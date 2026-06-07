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
    active_shortcut_id: Option<String>,
}

impl RecordingService {
    pub fn phase(&self) -> RecordingPhase {
        self.phase
    }

    #[cfg(test)]
    pub fn active_shortcut_id(&self) -> Option<&str> {
        self.active_shortcut_id.as_deref()
    }

    pub fn start_recording(&mut self, shortcut_id: &str) -> RecordingPhase {
        match self.phase {
            RecordingPhase::Idle => {
                self.phase = RecordingPhase::Arming;
                self.active_shortcut_id = Some(shortcut_id.to_string());
                start_recording(shortcut_id);
            }
            RecordingPhase::Arming => {
                info!(
                    active_shortcut_id = self.active_shortcut_id.as_deref().unwrap_or("unknown"),
                    requested_shortcut_id = shortcut_id,
                    "Start recording requested while audio capture is arming"
                );
            }
            RecordingPhase::Recording => {
                info!(
                    active_shortcut_id = self.active_shortcut_id.as_deref().unwrap_or("unknown"),
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

    pub fn capture_started(&mut self, shortcut_id: &str) -> RecordingPhase {
        if self.active_shortcut_id.as_deref() != Some(shortcut_id) {
            info!(
                active_shortcut_id = self.active_shortcut_id.as_deref().unwrap_or("unknown"),
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

    pub fn start_failed(&mut self, shortcut_id: &str) -> RecordingPhase {
        if self.active_shortcut_id.as_deref() == Some(shortcut_id)
            && matches!(
                self.phase,
                RecordingPhase::Arming | RecordingPhase::Processing
            )
        {
            self.phase = RecordingPhase::Idle;
            self.active_shortcut_id = None;
        }

        self.phase
    }

    pub fn stop_recording(&mut self, shortcut_id: &str) -> (RecordingPhase, bool) {
        match self.phase {
            RecordingPhase::Idle => {
                info!(shortcut_id, "Stop recording requested while not recording");
                (self.phase, false)
            }
            RecordingPhase::Arming => {
                if self.active_shortcut_id.as_deref() == Some(shortcut_id) {
                    stop_recording(shortcut_id);
                    self.phase = RecordingPhase::Processing;
                    (self.phase, true)
                } else {
                    info!(
                        active_shortcut_id =
                            self.active_shortcut_id.as_deref().unwrap_or("unknown"),
                        requested_shortcut_id = shortcut_id,
                        "Stop recording ignored while inactive shortcut is arming"
                    );
                    (self.phase, false)
                }
            }
            RecordingPhase::Recording => {
                if self.active_shortcut_id.as_deref() == Some(shortcut_id) {
                    stop_recording(shortcut_id);
                    self.phase = RecordingPhase::Processing;
                    (self.phase, true)
                } else {
                    info!(
                        active_shortcut_id =
                            self.active_shortcut_id.as_deref().unwrap_or("unknown"),
                        requested_shortcut_id = shortcut_id,
                        "Stop recording ignored for inactive shortcut"
                    );
                    (self.phase, false)
                }
            }
            RecordingPhase::Processing => {
                info!(
                    shortcut_id,
                    "Stop recording requested while processing audio"
                );
                (self.phase, false)
            }
        }
    }

    pub fn finish_processing<T>(
        &mut self,
        shortcut_id: &str,
        result: &Result<T, String>,
    ) -> RecordingPhase {
        if self.phase != RecordingPhase::Processing {
            info!(
                shortcut_id,
                "Transcription finished while recording service was not processing"
            );
            return self.phase;
        }

        if self.active_shortcut_id.as_deref() != Some(shortcut_id) {
            info!(
                active_shortcut_id = self.active_shortcut_id.as_deref().unwrap_or("unknown"),
                finished_shortcut_id = shortcut_id,
                "Transcription finished for inactive shortcut"
            );
            return self.phase;
        }

        if let Err(error) = result {
            warn!(shortcut_id, error, "Transcription failed");
        }

        self.phase = RecordingPhase::Idle;
        self.active_shortcut_id = None;
        self.phase
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
        assert_eq!(service.start_recording("default"), RecordingPhase::Arming);
        assert_eq!(
            service.capture_started("default"),
            RecordingPhase::Recording
        );
        assert_eq!(
            service.start_recording("default"),
            RecordingPhase::Recording
        );

        let (phase, should_stop) = service.stop_recording("default");
        assert_eq!(phase, RecordingPhase::Processing);
        assert!(should_stop);
        assert_eq!(
            service.stop_recording("default").0,
            RecordingPhase::Processing
        );
        assert_eq!(
            service.finish_processing("default", &Ok(())),
            RecordingPhase::Idle
        );
        assert_eq!(service.stop_recording("default").0, RecordingPhase::Idle);
    }

    #[test]
    fn processing_finishes_only_for_active_shortcut() {
        let mut service = RecordingService::default();

        assert_eq!(
            service.finish_processing("default", &Ok(())),
            RecordingPhase::Idle
        );
        assert_eq!(service.start_recording("default"), RecordingPhase::Arming);
        assert_eq!(
            service.capture_started("default"),
            RecordingPhase::Recording
        );
        assert_eq!(
            service.stop_recording("default").0,
            RecordingPhase::Processing
        );
        assert_eq!(
            service.finish_processing("second", &Ok(())),
            RecordingPhase::Processing
        );
        assert_eq!(
            service.finish_processing("default", &Ok(())),
            RecordingPhase::Idle
        );
    }

    #[test]
    fn ignores_stop_for_inactive_shortcut() {
        let mut service = RecordingService::default();
        service.start_recording("default");
        service.capture_started("default");

        let (phase, should_stop) = service.stop_recording("second");

        assert_eq!(phase, RecordingPhase::Recording);
        assert!(!should_stop);
        assert_eq!(service.active_shortcut_id(), Some("default"));
    }

    #[test]
    fn stop_while_arming_moves_to_processing() {
        let mut service = RecordingService::default();
        service.start_recording("default");

        let (phase, should_stop) = service.stop_recording("default");

        assert_eq!(phase, RecordingPhase::Processing);
        assert!(should_stop);
        assert_eq!(
            service.capture_started("default"),
            RecordingPhase::Processing
        );
    }

    #[test]
    fn start_failure_resets_arming_recording() {
        let mut service = RecordingService::default();
        service.start_recording("default");

        assert_eq!(service.start_failed("default"), RecordingPhase::Idle);
        assert_eq!(service.active_shortcut_id(), None);
    }
}
