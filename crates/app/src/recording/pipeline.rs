use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use shared::AudioInputRef;
use tracing::{debug, info, warn};

use crate::audio::AudioCaptureService;
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
            .name("myapp-audio-capture".to_string())
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

    pub fn prepare_input(&self, input: AudioInputRef) -> Result<()> {
        self.worker_tx
            .send(RecordingWorkerCommand::PrepareInput(input))
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
    PrepareInput(AudioInputRef),
    Start(Box<TranscriptionPlan>),
    Stop {
        recording_id: u64,
        shortcut_id: String,
    },
    Shutdown,
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
                    RecordingWorkerCommand::PrepareInput(input) => worker.prepare_input(&input),
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
    capture: Option<AudioCaptureService>,
    active_plan: Option<TranscriptionPlan>,
    active_samples: Vec<f32>,
}

impl RecordingWorker {
    fn new(command_tx: mpsc::Sender<AppCommand>) -> Self {
        Self {
            command_tx,
            capture: None,
            active_plan: None,
            active_samples: Vec::new(),
        }
    }

    fn prepare_input(&mut self, input: &AudioInputRef) {
        if let Some(plan) = &self.active_plan {
            debug!(
                active_shortcut_id = %plan.shortcut_id,
                input = %input.display_label(),
                "audio capture prepare ignored while recording is active"
            );
            return;
        }

        match self.capture_for_input(input) {
            Ok(capture) => info!(
                input = %capture.input_label(),
                "audio capture stream prepared"
            ),
            Err(error) => warn!(
                ?error,
                input = %input.display_label(),
                "failed to prepare audio capture stream"
            ),
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
        let capture = self.capture_for_input(&plan.input)?;
        let info = capture.start_session()?;
        self.active_samples.clear();
        self.active_plan = Some(plan);
        Ok(AppCommand::AudioCaptureStarted {
            recording_id,
            shortcut_id,
            input_label: info.input_label,
            startup_latency_ms: info.startup_latency_ms,
            first_callback_latency_ms: info.first_callback_latency_ms,
        })
    }

    fn capture_for_input(&mut self, input: &AudioInputRef) -> Result<&mut AudioCaptureService> {
        let recreate = self
            .capture
            .as_ref()
            .is_none_or(|capture| capture.input() != input);

        if recreate {
            if let Some(capture) = self.capture.take() {
                debug!(
                    old_input = %capture.input_label(),
                    new_input = %input.display_label(),
                    "recreating audio capture stream for input change"
                );
            }
            self.capture = Some(AudioCaptureService::for_input(input)?);
        }

        self.capture
            .as_mut()
            .ok_or_else(|| anyhow!("audio capture stream was not initialized"))
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

        let result = match self.capture.as_mut() {
            Some(capture) => {
                let stopped = capture.stop_session_with_samples(active_samples);
                if let Some(error) = &stopped.pause_error {
                    warn!(
                        error,
                        input = %stopped.audio.input_label,
                        "audio capture stream pause failed; recreating stream before next recording"
                    );
                }
                let pause_failed = stopped.pause_error.is_some();
                let request = plan.into_request(stopped.audio);
                if pause_failed {
                    self.capture.take();
                }
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
