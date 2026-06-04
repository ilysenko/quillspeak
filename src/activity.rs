use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use arboard::Clipboard;

use crate::audio::AudioRecorder;
use crate::tray::TrayBackend;
use crate::whisper::WhisperRecognizer;

const WHISPER_SAMPLE_RATE: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceActivityState {
    Idle,
    Recording,
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PushToTalkEvent {
    Pressed,
    Released,
}

#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    result: std::result::Result<String, String>,
}

impl TranscriptionResult {
    fn success(text: String) -> Self {
        Self { result: Ok(text) }
    }

    fn failure(error: String) -> Self {
        Self { result: Err(error) }
    }
}

pub type TranscriptionResultSender = Arc<dyn Fn(TranscriptionResult) + Send + Sync + 'static>;

pub trait ClipboardWriter {
    fn copy_text(&self, text: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct SystemClipboardWriter;

impl ClipboardWriter for SystemClipboardWriter {
    fn copy_text(&self, text: &str) -> Result<()> {
        Clipboard::new()
            .context("failed to initialize system clipboard")?
            .set_text(text.to_owned())
            .context("failed to write text to system clipboard")?;
        Ok(())
    }
}

pub struct VoiceActivityController {
    state: RefCell<VoiceActivityState>,
    tray_backend: Rc<dyn TrayBackend>,
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
    clipboard_writer: Rc<dyn ClipboardWriter>,
    transcription_result_sender: TranscriptionResultSender,
}

impl VoiceActivityController {
    pub fn new(
        tray_backend: Rc<dyn TrayBackend>,
        audio_recorder: Arc<dyn AudioRecorder>,
        whisper_recognizer: Arc<dyn WhisperRecognizer>,
        clipboard_writer: Rc<dyn ClipboardWriter>,
        transcription_result_sender: TranscriptionResultSender,
    ) -> Self {
        tray_backend.set_visual_state(VoiceActivityState::Idle);

        Self {
            state: RefCell::new(VoiceActivityState::Idle),
            tray_backend,
            audio_recorder,
            whisper_recognizer,
            clipboard_writer,
            transcription_result_sender,
        }
    }

    pub fn state(&self) -> VoiceActivityState {
        *self.state.borrow()
    }

    pub fn handle_push_to_talk_event(&self, event: PushToTalkEvent) -> Result<()> {
        match event {
            PushToTalkEvent::Pressed => self.begin_recording(),
            PushToTalkEvent::Released => self.finish_recording(),
        }
    }

    pub fn begin_recording(&self) -> Result<()> {
        match self.state() {
            VoiceActivityState::Recording | VoiceActivityState::Processing => {
                return Ok(());
            }
            VoiceActivityState::Idle => {}
        }

        self.set_state(VoiceActivityState::Recording);
        if let Err(error) = self.audio_recorder.start() {
            self.set_state(VoiceActivityState::Idle);
            return Err(error).context("failed to start audio recording");
        }

        Ok(())
    }

    pub fn finish_recording(&self) -> Result<()> {
        if self.state() != VoiceActivityState::Recording {
            return Ok(());
        }

        self.set_state(VoiceActivityState::Processing);

        let audio_recorder = Arc::clone(&self.audio_recorder);
        let whisper_recognizer = Arc::clone(&self.whisper_recognizer);
        let result_sender = Arc::clone(&self.transcription_result_sender);
        if let Err(error) = thread::Builder::new()
            .name("voice-whisper-transcription".to_string())
            .spawn(move || {
                let result = stop_and_transcribe(audio_recorder, whisper_recognizer)
                    .map(TranscriptionResult::success)
                    .unwrap_or_else(|error| TranscriptionResult::failure(format!("{error:#}")));
                result_sender(result);
            })
        {
            self.set_state(VoiceActivityState::Idle);
            return Err(error).context("failed to spawn Whisper transcription worker");
        }

        Ok(())
    }

    pub fn handle_transcription_result(&self, result: TranscriptionResult) -> Result<()> {
        let result = self.handle_transcription_result_inner(result);
        self.set_state(VoiceActivityState::Idle);
        result
    }

    fn handle_transcription_result_inner(&self, result: TranscriptionResult) -> Result<()> {
        let text = result.result.map_err(|error| anyhow!("{error}"))?;
        let text = text.trim();
        eprintln!("Recognized text: {}", display_transcription_text(text));
        if text.is_empty() {
            if voice_debug_enabled() {
                eprintln!("Transcription result was empty; clipboard was not changed.");
            }
            return Ok(());
        }

        self.clipboard_writer
            .copy_text(text)
            .context("failed to copy transcription to clipboard")?;
        eprintln!("Clipboard updated.");
        if voice_debug_enabled() {
            eprintln!(
                "Copied transcription to clipboard ({} chars).",
                text.chars().count()
            );
        }

        Ok(())
    }

    fn set_state(&self, state: VoiceActivityState) {
        self.state.replace(state);
        self.tray_backend.set_visual_state(state);
    }
}

fn stop_and_transcribe(
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
) -> Result<String> {
    let samples = audio_recorder
        .stop()
        .context("failed to stop audio recording")?;
    if samples.is_empty() {
        if voice_debug_enabled() {
            eprintln!("No audio samples captured yet; skipping Whisper transcription.");
        }
        return Ok(String::new());
    }

    if voice_debug_enabled() {
        log_audio_stats(&samples);
    }
    whisper_recognizer
        .transcribe(&samples)
        .context("failed to transcribe recorded audio")
}

fn voice_debug_enabled() -> bool {
    std::env::var_os("VOICE_DEBUG").is_some()
}

fn display_transcription_text(text: &str) -> &str {
    if text.is_empty() { "<empty>" } else { text }
}

fn log_audio_stats(samples: &[i16]) {
    let duration_secs = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;
    let peak = samples
        .iter()
        .map(|sample| i32::from(*sample).abs())
        .max()
        .unwrap_or_default() as f32
        / i16::MAX as f32;
    let rms = (samples
        .iter()
        .map(|sample| {
            let normalized = *sample as f64 / i16::MAX as f64;
            normalized * normalized
        })
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt();

    eprintln!(
        "Captured audio: {:.2}s, {} samples @ 16 kHz, peak {:.3}, rms {:.4}.",
        duration_secs,
        samples.len(),
        peak,
        rms
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use anyhow::{Result, anyhow};

    use super::*;
    use crate::tray::TrayBackend;
    use crate::whisper::{
        WhisperBackend, WhisperBackendPreference, WhisperRecognizer, WhisperRuntimeStatus,
    };

    #[derive(Default)]
    struct MockTrayBackend {
        states: RefCell<Vec<VoiceActivityState>>,
    }

    impl TrayBackend for MockTrayBackend {
        fn backend_name(&self) -> &'static str {
            "mock"
        }

        fn set_visual_state(&self, state: VoiceActivityState) {
            self.states.borrow_mut().push(state);
        }
    }

    struct MockRecorder {
        started: AtomicU32,
        stopped: AtomicU32,
        samples: Vec<i16>,
        stop_error: Option<String>,
        stop_delay: Duration,
    }

    impl Default for MockRecorder {
        fn default() -> Self {
            Self {
                started: AtomicU32::new(0),
                stopped: AtomicU32::new(0),
                samples: vec![0, 1, -1],
                stop_error: None,
                stop_delay: Duration::ZERO,
            }
        }
    }

    impl AudioRecorder for MockRecorder {
        fn start(&self) -> Result<()> {
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn stop(&self) -> Result<Vec<i16>> {
            if !self.stop_delay.is_zero() {
                std::thread::sleep(self.stop_delay);
            }
            self.stopped.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.stop_error.as_deref() {
                return Err(anyhow!("{error}"));
            }
            Ok(self.samples.clone())
        }

        fn configure_input_device(&self, _device_name: Option<&str>) -> Result<()> {
            Ok(())
        }

        fn available_input_devices(&self) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    struct MockRecognizer {
        text: String,
        fail: bool,
        transcribed: AtomicU32,
    }

    impl WhisperRecognizer for MockRecognizer {
        fn configure_model(
            &self,
            _model: &str,
            _backend: WhisperBackendPreference,
            _gpu_device: i32,
        ) -> Result<()> {
            Ok(())
        }

        fn transcribe(&self, _samples: &[i16]) -> Result<String> {
            self.transcribed.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(anyhow!("recognizer failed"))
            } else {
                Ok(self.text.clone())
            }
        }

        fn runtime_status(&self) -> WhisperRuntimeStatus {
            WhisperRuntimeStatus::default()
        }

        fn available_backends(&self) -> Vec<WhisperBackend> {
            vec![WhisperBackend::Cpu]
        }
    }

    #[derive(Default)]
    struct ActivityHarness {
        results: Arc<Mutex<Vec<TranscriptionResult>>>,
    }

    impl ActivityHarness {
        fn sender(&self) -> TranscriptionResultSender {
            let results = Arc::clone(&self.results);
            Arc::new(move |result| {
                results
                    .lock()
                    .expect("test transcription results were poisoned")
                    .push(result);
            })
        }

        fn next_result(&self) -> TranscriptionResult {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = self
                    .results
                    .lock()
                    .expect("test transcription results were poisoned")
                    .pop()
                {
                    return result;
                }
                if Instant::now() >= deadline {
                    panic!("timed out waiting for transcription result");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[derive(Default)]
    struct MockClipboard {
        copied: RefCell<Vec<String>>,
    }

    impl ClipboardWriter for MockClipboard {
        fn copy_text(&self, text: &str) -> Result<()> {
            self.copied.borrow_mut().push(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn successful_recording_flow_sets_recording_processing_idle() {
        let tray = Rc::new(MockTrayBackend::default());
        let recorder = Arc::new(MockRecorder::default());
        let recognizer = Arc::new(MockRecognizer {
            text: " hello ".to_string(),
            fail: false,
            transcribed: AtomicU32::new(0),
        });
        let clipboard = Rc::new(MockClipboard::default());
        let harness = ActivityHarness::default();

        let controller = VoiceActivityController::new(
            tray.clone(),
            recorder.clone(),
            recognizer.clone(),
            clipboard.clone(),
            harness.sender(),
        );

        controller
            .handle_push_to_talk_event(PushToTalkEvent::Pressed)
            .unwrap();
        controller
            .handle_push_to_talk_event(PushToTalkEvent::Released)
            .unwrap();
        assert_eq!(controller.state(), VoiceActivityState::Processing);
        let result = harness.next_result();
        controller.handle_transcription_result(result).unwrap();

        assert_eq!(
            tray.states.borrow().as_slice(),
            &[
                VoiceActivityState::Idle,
                VoiceActivityState::Recording,
                VoiceActivityState::Processing,
                VoiceActivityState::Idle,
            ]
        );
        assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 1);
        assert_eq!(recognizer.transcribed.load(Ordering::SeqCst), 1);
        assert_eq!(clipboard.copied.borrow().as_slice(), &["hello"]);
    }

    #[test]
    fn failed_processing_returns_to_idle() {
        let tray = Rc::new(MockTrayBackend::default());
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray.clone(),
            Arc::new(MockRecorder::default()),
            Arc::new(MockRecognizer {
                text: String::new(),
                fail: true,
                transcribed: AtomicU32::new(0),
            }),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        controller.finish_recording().unwrap();
        let result = harness.next_result();
        assert!(controller.handle_transcription_result(result).is_err());

        assert_eq!(controller.state(), VoiceActivityState::Idle);
        assert_eq!(
            tray.states.borrow().as_slice(),
            &[
                VoiceActivityState::Idle,
                VoiceActivityState::Recording,
                VoiceActivityState::Processing,
                VoiceActivityState::Idle,
            ]
        );
    }

    #[test]
    fn empty_transcription_does_not_touch_clipboard() {
        let clipboard = Rc::new(MockClipboard::default());
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            Rc::new(MockTrayBackend::default()),
            Arc::new(MockRecorder::default()),
            Arc::new(MockRecognizer {
                text: "   ".to_string(),
                fail: false,
                transcribed: AtomicU32::new(0),
            }),
            clipboard.clone(),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        controller.finish_recording().unwrap();
        let result = harness.next_result();
        controller.handle_transcription_result(result).unwrap();

        assert!(clipboard.copied.borrow().is_empty());
        assert_eq!(controller.state(), VoiceActivityState::Idle);
    }

    #[test]
    fn empty_recording_samples_skip_processing_and_transcription() {
        let tray = Rc::new(MockTrayBackend::default());
        let mut recorder = MockRecorder::default();
        recorder.samples = Vec::new();
        let recorder = Arc::new(recorder);
        let recognizer = Arc::new(MockRecognizer {
            text: "should not run".to_string(),
            fail: false,
            transcribed: AtomicU32::new(0),
        });
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray.clone(),
            recorder.clone(),
            recognizer.clone(),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        controller.finish_recording().unwrap();
        assert_eq!(controller.state(), VoiceActivityState::Processing);
        let result = harness.next_result();
        controller.handle_transcription_result(result).unwrap();

        assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 1);
        assert_eq!(recognizer.transcribed.load(Ordering::SeqCst), 0);
        assert_eq!(controller.state(), VoiceActivityState::Idle);
        assert_eq!(
            tray.states.borrow().as_slice(),
            &[
                VoiceActivityState::Idle,
                VoiceActivityState::Recording,
                VoiceActivityState::Processing,
                VoiceActivityState::Idle,
            ]
        );
    }

    #[test]
    fn pressed_during_processing_is_ignored() {
        let tray = Rc::new(MockTrayBackend::default());
        let recorder = Arc::new(MockRecorder::default());
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray,
            recorder.clone(),
            Arc::new(MockRecognizer {
                text: "hello".to_string(),
                fail: false,
                transcribed: AtomicU32::new(0),
            }),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        controller.finish_recording().unwrap();
        controller.begin_recording().unwrap();

        assert_eq!(controller.state(), VoiceActivityState::Processing);
        assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repeated_press_while_recording_is_ignored_as_key_autorepeat() {
        let tray = Rc::new(MockTrayBackend::default());
        let recorder = Arc::new(MockRecorder::default());
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray.clone(),
            recorder.clone(),
            Arc::new(MockRecognizer {
                text: "hello".to_string(),
                fail: false,
                transcribed: AtomicU32::new(0),
            }),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller
            .handle_push_to_talk_event(PushToTalkEvent::Pressed)
            .unwrap();
        controller
            .handle_push_to_talk_event(PushToTalkEvent::Pressed)
            .unwrap();

        assert_eq!(controller.state(), VoiceActivityState::Recording);
        assert_eq!(recorder.started.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 0);
        assert!(harness.results.lock().unwrap().is_empty());
        assert_eq!(
            tray.states.borrow().as_slice(),
            &[VoiceActivityState::Idle, VoiceActivityState::Recording]
        );
    }

    #[test]
    fn release_switches_to_processing_before_audio_stop_finishes() {
        let tray = Rc::new(MockTrayBackend::default());
        let mut recorder = MockRecorder::default();
        recorder.stop_delay = Duration::from_millis(150);
        let recorder = Arc::new(recorder);
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray,
            recorder.clone(),
            Arc::new(MockRecognizer {
                text: "hello".to_string(),
                fail: false,
                transcribed: AtomicU32::new(0),
            }),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        let released_at = Instant::now();
        controller.finish_recording().unwrap();

        assert!(released_at.elapsed() < Duration::from_millis(100));
        assert_eq!(controller.state(), VoiceActivityState::Processing);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 0);

        let result = harness.next_result();
        controller.handle_transcription_result(result).unwrap();
        assert_eq!(controller.state(), VoiceActivityState::Idle);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn audio_stop_failure_returns_to_idle_through_worker_result() {
        let tray = Rc::new(MockTrayBackend::default());
        let mut recorder = MockRecorder::default();
        recorder.stop_error = Some("stop failed".to_string());
        let recorder = Arc::new(recorder);
        let recognizer = Arc::new(MockRecognizer {
            text: "should not run".to_string(),
            fail: false,
            transcribed: AtomicU32::new(0),
        });
        let harness = ActivityHarness::default();
        let controller = VoiceActivityController::new(
            tray.clone(),
            recorder.clone(),
            recognizer.clone(),
            Rc::new(MockClipboard::default()),
            harness.sender(),
        );

        controller.begin_recording().unwrap();
        controller.finish_recording().unwrap();
        assert_eq!(controller.state(), VoiceActivityState::Processing);
        let result = harness.next_result();
        assert!(controller.handle_transcription_result(result).is_err());

        assert_eq!(controller.state(), VoiceActivityState::Idle);
        assert_eq!(recorder.stopped.load(Ordering::SeqCst), 1);
        assert_eq!(recognizer.transcribed.load(Ordering::SeqCst), 0);
        assert_eq!(
            tray.states.borrow().as_slice(),
            &[
                VoiceActivityState::Idle,
                VoiceActivityState::Recording,
                VoiceActivityState::Processing,
                VoiceActivityState::Idle,
            ]
        );
    }
}
