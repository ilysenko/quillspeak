use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use shared::AudioInputRef;
use tracing::{debug, info, warn};

use crate::audio::{AudioCaptureService, AudioCaptureStartInfo, AudioCaptureStopInfo};
use crate::command::AppCommand;
use crate::transcription::TranscriptionPlan;

const CAPTURE_DRAIN_INTERVAL: Duration = Duration::from_millis(20);

pub struct RecordingPipeline {
    worker_tx: mpsc::Sender<RecordingWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl RecordingPipeline {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("quillspeak-audio-capture".to_string())
            .spawn(move || recording_worker_loop(worker_rx, command_tx))
            .map_err(|error| anyhow!("failed to spawn audio capture worker: {error}"))?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn start(&self, plan: TranscriptionPlan) -> Result<()> {
        self.worker_tx
            .send(RecordingWorkerCommand::Start(Box::new(plan)))
            .map_err(|_| anyhow!("audio capture worker is not running"))
    }

    pub fn stop(&self, recording_id: u64, shortcut_id: &str) -> Result<()> {
        self.worker_tx
            .send(RecordingWorkerCommand::Stop {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
            })
            .map_err(|_| anyhow!("audio capture worker is not running"))
    }

    pub fn shutdown(mut self) {
        let _ = self.worker_tx.send(RecordingWorkerCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "audio capture worker panicked during shutdown");
        }
    }
}

enum RecordingWorkerCommand {
    Start(Box<TranscriptionPlan>),
    Stop {
        recording_id: u64,
        shortcut_id: String,
    },
    Shutdown,
}

trait CaptureSession {
    fn input_label(&self) -> &str;
    fn start_session(&mut self) -> Result<AudioCaptureStartInfo>;
    fn stop_session(&mut self) -> AudioCaptureStopInfo;
    fn stop_session_with_samples(&mut self, samples: Vec<f32>) -> AudioCaptureStopInfo;
    fn collect_available_samples(&mut self, samples: &mut Vec<f32>);
}

impl CaptureSession for AudioCaptureService {
    fn input_label(&self) -> &str {
        self.input_label()
    }

    fn start_session(&mut self) -> Result<AudioCaptureStartInfo> {
        AudioCaptureService::start_session(self)
    }

    fn stop_session(&mut self) -> AudioCaptureStopInfo {
        AudioCaptureService::stop_session(self)
    }

    fn stop_session_with_samples(&mut self, samples: Vec<f32>) -> AudioCaptureStopInfo {
        AudioCaptureService::stop_session_with_samples(self, samples)
    }

    fn collect_available_samples(&mut self, samples: &mut Vec<f32>) {
        AudioCaptureService::collect_available_samples(self, samples);
    }
}

type CaptureFactory = Box<dyn FnMut(&AudioInputRef) -> Result<Box<dyn CaptureSession>>>;

fn default_capture_factory() -> CaptureFactory {
    Box::new(|input| {
        let capture: Box<dyn CaptureSession> = Box::new(AudioCaptureService::for_input(input)?);
        Ok(capture)
    })
}

fn recording_worker_loop(
    worker_rx: mpsc::Receiver<RecordingWorkerCommand>,
    command_tx: mpsc::Sender<AppCommand>,
) {
    let mut worker = RecordingWorker::new(command_tx);
    loop {
        match worker_rx.recv_timeout(CAPTURE_DRAIN_INTERVAL) {
            Ok(command) => {
                let shutdown = matches!(command, RecordingWorkerCommand::Shutdown);
                match command {
                    RecordingWorkerCommand::Start(plan) => worker.start(*plan),
                    RecordingWorkerCommand::Stop {
                        recording_id,
                        shortcut_id,
                    } => worker.stop(recording_id, &shortcut_id),
                    RecordingWorkerCommand::Shutdown => worker.cancel(),
                }
                worker.collect_active_samples();
                if shutdown {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => worker.collect_active_samples(),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                worker.cancel();
                break;
            }
        }
    }
}

struct RecordingWorker {
    command_tx: mpsc::Sender<AppCommand>,
    capture: Option<Box<dyn CaptureSession>>,
    capture_factory: CaptureFactory,
    active_plan: Option<TranscriptionPlan>,
    active_samples: Vec<f32>,
}

impl RecordingWorker {
    fn new(command_tx: mpsc::Sender<AppCommand>) -> Self {
        Self::with_capture_factory(command_tx, default_capture_factory())
    }

    fn with_capture_factory(
        command_tx: mpsc::Sender<AppCommand>,
        capture_factory: CaptureFactory,
    ) -> Self {
        Self {
            command_tx,
            capture: None,
            capture_factory,
            active_plan: None,
            active_samples: Vec::new(),
        }
    }

    fn start(&mut self, plan: TranscriptionPlan) {
        let shortcut_id = plan.shortcut_id.clone();
        let recording_id = plan.recording_id;
        if self.active_plan.is_some() {
            warn!(
                recording_id,
                shortcut_id, "audio capture start ignored because another recording is active"
            );
            self.send(AppCommand::AudioCaptureStartFailed {
                recording_id,
                shortcut_id,
                error: "audio capture is already active".to_string(),
            });
            return;
        }

        match self.start_inner(plan) {
            Ok(started) => self.send(started),
            Err(error) => {
                warn!(
                    ?error,
                    recording_id, shortcut_id, "failed to start audio capture"
                );
                self.active_plan = None;
                self.active_samples.clear();
                self.capture.take();
                self.send(AppCommand::AudioCaptureStartFailed {
                    recording_id,
                    shortcut_id,
                    error: format!("{error:#}"),
                });
            }
        }
    }

    fn start_inner(&mut self, plan: TranscriptionPlan) -> Result<AppCommand> {
        let shortcut_id = plan.shortcut_id.clone();
        let recording_id = plan.recording_id;
        let mut capture = (self.capture_factory)(&plan.input)?;
        debug!(
            input = %capture.input_label(),
            "audio capture stream opened for recording"
        );
        let info = capture.start_session()?;
        self.active_samples.clear();
        self.active_plan = Some(plan);
        self.capture = Some(capture);
        Ok(AppCommand::AudioCaptureStarted {
            recording_id,
            shortcut_id,
            input_label: info.input_label,
            startup_latency_ms: info.startup_latency_ms,
            first_callback_latency_ms: info.first_callback_latency_ms,
        })
    }

    fn stop(&mut self, recording_id: u64, shortcut_id: &str) {
        let Some(plan) = self.active_plan.as_ref() else {
            info!(
                recording_id,
                shortcut_id, "audio capture stop ignored because no recording is active"
            );
            return;
        };

        if plan.recording_id != recording_id || plan.shortcut_id != shortcut_id {
            warn!(
                active_recording_id = plan.recording_id,
                active_shortcut_id = %plan.shortcut_id,
                requested_recording_id = recording_id,
                requested_shortcut_id = shortcut_id,
                "audio capture stop ignored for inactive shortcut"
            );
            return;
        }
        self.collect_active_samples();
        let plan = self
            .active_plan
            .take()
            .expect("active plan was checked before stop");
        let active_samples = std::mem::take(&mut self.active_samples);

        let result = match self.capture.take() {
            Some(mut capture) => {
                let stopped = capture.stop_session_with_samples(active_samples);
                if let Some(error) = &stopped.pause_error {
                    warn!(
                        error,
                        input = %stopped.audio.input_label,
                        "audio capture stream pause failed before release"
                    );
                }
                debug!(
                    input = %stopped.audio.input_label,
                    "audio capture stream released after recording"
                );
                let request = plan.into_request(stopped.audio);
                Ok(request)
            }
            None => Err(anyhow!("audio capture stream was not initialized")),
        }
        .map_err(|error| format!("{error:#}"));

        self.send(AppCommand::AudioCaptureStopped {
            recording_id,
            shortcut_id: shortcut_id.to_string(),
            result: result.map(Box::new),
        });
    }

    fn cancel(&mut self) {
        self.active_plan.take();
        self.active_samples.clear();
        if let Some(mut capture) = self.capture.take() {
            let stopped = capture.stop_session();
            if let Some(error) = stopped.pause_error {
                warn!(
                    error,
                    input = %stopped.audio.input_label,
                    "audio capture stream pause failed during cancellation"
                );
            }
            debug!(
                input = %stopped.audio.input_label,
                "audio capture stream released during cancellation"
            );
        }
    }

    fn collect_active_samples(&mut self) {
        if self.active_plan.is_some()
            && let Some(capture) = self.capture.as_mut()
        {
            capture.collect_available_samples(&mut self.active_samples);
        }
    }

    fn send(&self, command: AppCommand) {
        let _ = self.command_tx.send(command);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::Instant;

    use shared::{
        AudioInputRef, ComputeBackend, DEFAULT_MODEL_ID, DEFAULT_SHORTCUT_ID, OutputAction,
    };

    use super::*;
    use crate::audio::CapturedAudio;

    #[test]
    fn stop_releases_capture_stream_after_recording() {
        let (command_tx, command_rx) = mpsc::channel();
        let state = Rc::new(FakeCaptureState::default());
        let mut worker =
            RecordingWorker::with_capture_factory(command_tx, fake_capture_factory(&state));

        worker.start(test_plan(7));
        assert_eq!(state.created.get(), 1);
        assert_eq!(state.dropped.get(), 0);
        assert!(worker.capture.is_some());
        assert!(matches!(
            command_rx.recv().expect("start command should be sent"),
            AppCommand::AudioCaptureStarted {
                recording_id: 7,
                ..
            }
        ));

        worker.stop(7, DEFAULT_SHORTCUT_ID);

        assert_eq!(state.stopped.get(), 1);
        assert_eq!(state.dropped.get(), 1);
        assert!(worker.capture.is_none());
        match command_rx.recv().expect("stop command should be sent") {
            AppCommand::AudioCaptureStopped {
                recording_id,
                shortcut_id,
                result,
            } => {
                assert_eq!(recording_id, 7);
                assert_eq!(shortcut_id, DEFAULT_SHORTCUT_ID);
                let request = result.expect("stop should produce a transcription request");
                assert_eq!(request.audio.samples, vec![0.25]);
            }
            command => panic!("unexpected command after stop: {command:?}"),
        }
    }

    #[test]
    fn failed_start_releases_capture_stream() {
        let (command_tx, command_rx) = mpsc::channel();
        let state = Rc::new(FakeCaptureState::default());
        state.fail_start.set(true);
        let mut worker =
            RecordingWorker::with_capture_factory(command_tx, fake_capture_factory(&state));

        worker.start(test_plan(11));

        assert_eq!(state.created.get(), 1);
        assert_eq!(state.dropped.get(), 1);
        assert!(worker.capture.is_none());
        assert!(worker.active_plan.is_none());
        match command_rx
            .recv()
            .expect("start failure command should be sent")
        {
            AppCommand::AudioCaptureStartFailed {
                recording_id,
                shortcut_id,
                error,
            } => {
                assert_eq!(recording_id, 11);
                assert_eq!(shortcut_id, DEFAULT_SHORTCUT_ID);
                assert!(error.contains("fake start failure"));
            }
            command => panic!("unexpected command after start failure: {command:?}"),
        }
    }

    fn fake_capture_factory(state: &Rc<FakeCaptureState>) -> CaptureFactory {
        let state = Rc::clone(state);
        Box::new(move |_| {
            state.created.set(state.created.get() + 1);
            Ok(Box::new(FakeCapture {
                state: Rc::clone(&state),
                input_label: "Fake input".to_string(),
            }))
        })
    }

    #[derive(Default)]
    struct FakeCaptureState {
        created: Cell<usize>,
        dropped: Cell<usize>,
        stopped: Cell<usize>,
        fail_start: Cell<bool>,
    }

    struct FakeCapture {
        state: Rc<FakeCaptureState>,
        input_label: String,
    }

    impl CaptureSession for FakeCapture {
        fn input_label(&self) -> &str {
            &self.input_label
        }

        fn start_session(&mut self) -> Result<AudioCaptureStartInfo> {
            if self.state.fail_start.get() {
                anyhow::bail!("fake start failure");
            }
            Ok(AudioCaptureStartInfo {
                input_label: self.input_label.clone(),
                startup_latency_ms: 3,
                first_callback_latency_ms: Some(4),
            })
        }

        fn stop_session(&mut self) -> AudioCaptureStopInfo {
            self.stop_session_with_samples(Vec::new())
        }

        fn stop_session_with_samples(&mut self, samples: Vec<f32>) -> AudioCaptureStopInfo {
            self.state.stopped.set(self.state.stopped.get() + 1);
            let now = Instant::now();
            AudioCaptureStopInfo {
                audio: CapturedAudio {
                    samples,
                    sample_rate: 16_000,
                    channels: 1,
                    input_label: self.input_label.clone(),
                    started_at: now,
                    stopped_at: now,
                    startup_latency_ms: 3,
                    first_callback_latency_ms: Some(4),
                    audio_callback_count: 0,
                    dropped_samples: 0,
                    missed_chunks: 0,
                    stale_callback_count: 0,
                    stale_samples: 0,
                },
                pause_error: None,
            }
        }

        fn collect_available_samples(&mut self, samples: &mut Vec<f32>) {
            samples.push(0.25);
        }
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            self.state.dropped.set(self.state.dropped.get() + 1);
        }
    }

    fn test_plan(recording_id: u64) -> TranscriptionPlan {
        TranscriptionPlan {
            recording_id,
            shortcut_id: DEFAULT_SHORTCUT_ID.to_string(),
            shortcut_name: "Default".to_string(),
            model_id: DEFAULT_MODEL_ID.to_string(),
            model_path: PathBuf::from("/tmp/quillspeak-test-model.bin"),
            language: "auto".to_string(),
            compute_backend: ComputeBackend::Cpu,
            mute_output_while_recording: false,
            beep_on_recording: false,
            beep_volume_percent: 100,
            output: OutputAction::default(),
            input: AudioInputRef::SystemDefault,
        }
    }
}
