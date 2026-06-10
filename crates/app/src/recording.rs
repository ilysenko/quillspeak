use tracing::{info, warn};

mod pipeline;

pub use pipeline::RecordingPipeline;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecordingPhase {
    #[default]
    Idle,
    Arming,
    Recording,
    Processing,
}

#[derive(Debug, Default)]
pub struct RecordingService {
    phase: RecordingPhase,
    active: Option<ActiveRecording>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveRecording {
    id: u64,
    shortcut_id: String,
}

impl RecordingService {
    pub fn phase(&self) -> RecordingPhase {
        self.phase
    }

    pub fn active_shortcut_id(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|active| active.shortcut_id.as_str())
    }

    pub fn active_recording_id(&self) -> Option<u64> {
        self.active.as_ref().map(|active| active.id)
    }

    pub fn start_recording(&mut self, recording_id: u64, shortcut_id: &str) -> RecordingPhase {
        match self.phase {
            RecordingPhase::Idle => {
                self.phase = RecordingPhase::Arming;
                self.active = Some(ActiveRecording {
                    id: recording_id,
                    shortcut_id: shortcut_id.to_string(),
                });
                start_recording(shortcut_id);
            }
            RecordingPhase::Arming => {
                info!(
                    active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                    requested_shortcut_id = shortcut_id,
                    "Start recording requested while audio capture is arming"
                );
            }
            RecordingPhase::Recording => {
                info!(
                    active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                    requested_shortcut_id = shortcut_id,
                    "Start recording requested while already recording"
                );
            }
            RecordingPhase::Processing => {
                info!(
                    requested_shortcut_id = shortcut_id,
                    "Start recording requested while processing audio"
                );
            }
        }

        self.phase
    }

    pub fn capture_started(&mut self, recording_id: u64, shortcut_id: &str) -> RecordingPhase {
        if !self.matches_active(recording_id, shortcut_id) {
            info!(
                active_recording_id = self.active.as_ref().map(|active| active.id).unwrap_or(0),
                active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                started_recording_id = recording_id,
                started_shortcut_id = shortcut_id,
                "Audio capture started for inactive shortcut"
            );
            return self.phase;
        }

        if self.phase == RecordingPhase::Arming {
            self.phase = RecordingPhase::Recording;
        }

        self.phase
    }

    pub fn start_failed(&mut self, recording_id: u64, shortcut_id: &str) -> RecordingPhase {
        if self.matches_active(recording_id, shortcut_id)
            && matches!(
                self.phase,
                RecordingPhase::Arming | RecordingPhase::Processing
            )
        {
            self.phase = RecordingPhase::Idle;
            self.active = None;
        }

        self.phase
    }

    pub fn cancel_arming(&mut self, recording_id: u64, shortcut_id: &str) -> RecordingPhase {
        if self.phase == RecordingPhase::Arming && self.matches_active(recording_id, shortcut_id) {
            info!(
                recording_id,
                shortcut_id, "Recording start canceled before audio capture"
            );
            self.phase = RecordingPhase::Idle;
            self.active = None;
        }

        self.phase
    }

    pub fn stop_recording(&mut self, shortcut_id: &str) -> (RecordingPhase, Option<u64>) {
        match self.phase {
            RecordingPhase::Idle => {
                info!(shortcut_id, "Stop recording requested while not recording");
                (self.phase, None)
            }
            RecordingPhase::Arming => {
                if self.active_shortcut_id() == Some(shortcut_id) {
                    let recording_id = self.active.as_ref().map(|active| active.id);
                    stop_recording(shortcut_id);
                    self.phase = RecordingPhase::Processing;
                    (self.phase, recording_id)
                } else {
                    info!(
                        active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                        requested_shortcut_id = shortcut_id,
                        "Stop recording ignored while inactive shortcut is arming"
                    );
                    (self.phase, None)
                }
            }
            RecordingPhase::Recording => {
                if self.active_shortcut_id() == Some(shortcut_id) {
                    let recording_id = self.active.as_ref().map(|active| active.id);
                    stop_recording(shortcut_id);
                    self.phase = RecordingPhase::Processing;
                    (self.phase, recording_id)
                } else {
                    info!(
                        active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                        requested_shortcut_id = shortcut_id,
                        "Stop recording ignored for inactive shortcut"
                    );
                    (self.phase, None)
                }
            }
            RecordingPhase::Processing => {
                info!(
                    shortcut_id,
                    "Stop recording requested while processing audio"
                );
                (self.phase, None)
            }
        }
    }

    pub fn is_processing(&self, recording_id: u64, shortcut_id: &str) -> bool {
        self.phase == RecordingPhase::Processing && self.matches_active(recording_id, shortcut_id)
    }

    pub fn finish_processing<T>(
        &mut self,
        recording_id: u64,
        shortcut_id: &str,
        result: &Result<T, String>,
    ) -> (RecordingPhase, bool) {
        if self.phase != RecordingPhase::Processing {
            info!(
                recording_id,
                shortcut_id, "Transcription finished while recording service was not processing"
            );
            return (self.phase, false);
        }

        if !self.matches_active(recording_id, shortcut_id) {
            info!(
                active_recording_id = self.active.as_ref().map(|active| active.id).unwrap_or(0),
                active_shortcut_id = self.active_shortcut_id().unwrap_or("unknown"),
                finished_recording_id = recording_id,
                finished_shortcut_id = shortcut_id,
                "Transcription finished for inactive shortcut"
            );
            return (self.phase, false);
        }

        if let Err(error) = result {
            warn!(shortcut_id, error, "Transcription failed");
        }

        self.phase = RecordingPhase::Idle;
        self.active = None;
        (self.phase, true)
    }

    fn matches_active(&self, recording_id: u64, shortcut_id: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.id == recording_id && active.shortcut_id == shortcut_id)
    }
}

pub fn start_recording(shortcut_id: &str) {
    info!(shortcut_id, "Start recording");
}

pub fn stop_recording(shortcut_id: &str) {
    info!(shortcut_id, "Stop recording");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_start_and_stop_are_safe() {
        let mut service = RecordingService::default();

        assert_eq!(service.phase(), RecordingPhase::Idle);
        assert_eq!(
            service.start_recording(1, "default"),
            RecordingPhase::Arming
        );
        assert_eq!(
            service.capture_started(1, "default"),
            RecordingPhase::Recording
        );
        assert_eq!(
            service.start_recording(2, "default"),
            RecordingPhase::Recording
        );

        let (phase, recording_id) = service.stop_recording("default");
        assert_eq!(phase, RecordingPhase::Processing);
        assert_eq!(recording_id, Some(1));
        assert_eq!(
            service.stop_recording("default").0,
            RecordingPhase::Processing
        );
        let (phase, accepted) = service.finish_processing(1, "default", &Ok(()));
        assert_eq!(phase, RecordingPhase::Idle);
        assert!(accepted);
        assert_eq!(service.stop_recording("default").0, RecordingPhase::Idle);
    }

    #[test]
    fn arming_recording_can_be_canceled_before_capture_starts() {
        let mut service = RecordingService::default();

        assert_eq!(
            service.start_recording(7, "default"),
            RecordingPhase::Arming
        );
        assert_eq!(service.cancel_arming(7, "default"), RecordingPhase::Idle);
        assert_eq!(service.active_recording_id(), None);
    }

    #[test]
    fn processing_finishes_only_for_active_shortcut() {
        let mut service = RecordingService::default();

        let (phase, accepted) = service.finish_processing(1, "default", &Ok(()));
        assert_eq!(phase, RecordingPhase::Idle);
        assert!(!accepted);
        assert_eq!(
            service.start_recording(1, "default"),
            RecordingPhase::Arming
        );
        assert_eq!(
            service.capture_started(1, "default"),
            RecordingPhase::Recording
        );
        assert_eq!(
            service.stop_recording("default").0,
            RecordingPhase::Processing
        );
        let (phase, accepted) = service.finish_processing(1, "second", &Ok(()));
        assert_eq!(phase, RecordingPhase::Processing);
        assert!(!accepted);
        let (phase, accepted) = service.finish_processing(1, "default", &Ok(()));
        assert_eq!(phase, RecordingPhase::Idle);
        assert!(accepted);
    }

    #[test]
    fn ignores_stop_for_inactive_shortcut() {
        let mut service = RecordingService::default();
        service.start_recording(1, "default");
        service.capture_started(1, "default");

        let (phase, recording_id) = service.stop_recording("second");

        assert_eq!(phase, RecordingPhase::Recording);
        assert_eq!(recording_id, None);
        assert_eq!(service.active_shortcut_id(), Some("default"));
    }

    #[test]
    fn stop_while_arming_moves_to_processing() {
        let mut service = RecordingService::default();
        service.start_recording(1, "default");

        let (phase, recording_id) = service.stop_recording("default");

        assert_eq!(phase, RecordingPhase::Processing);
        assert_eq!(recording_id, Some(1));
        assert_eq!(
            service.capture_started(1, "default"),
            RecordingPhase::Processing
        );
    }

    #[test]
    fn start_failure_resets_arming_recording() {
        let mut service = RecordingService::default();
        service.start_recording(1, "default");

        assert_eq!(service.start_failed(1, "default"), RecordingPhase::Idle);
        assert_eq!(service.active_shortcut_id(), None);
    }

    #[test]
    fn stale_transcription_result_is_rejected_by_recording_id() {
        let mut service = RecordingService::default();
        service.start_recording(1, "default");
        service.capture_started(1, "default");
        assert_eq!(service.stop_recording("default").1, Some(1));

        let (phase, accepted) = service.finish_processing(2, "default", &Ok(()));

        assert_eq!(phase, RecordingPhase::Processing);
        assert!(!accepted);
        assert_eq!(service.active_recording_id(), Some(1));
        assert_eq!(service.active_shortcut_id(), Some("default"));
    }

    #[test]
    fn transcription_error_finishes_active_processing() {
        let mut service = RecordingService::default();
        service.start_recording(1, "default");
        service.capture_started(1, "default");
        assert_eq!(service.stop_recording("default").1, Some(1));

        let (phase, accepted) =
            service.finish_processing::<()>(1, "default", &Err("transcription failed".into()));

        assert_eq!(phase, RecordingPhase::Idle);
        assert!(accepted);
        assert_eq!(service.active_recording_id(), None);
        assert_eq!(service.active_shortcut_id(), None);
    }
}
