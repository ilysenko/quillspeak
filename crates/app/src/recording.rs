use tracing::info;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecordingPhase {
    #[default]
    Idle,
    Recording,
    Processing,
}

#[derive(Debug, Default)]
pub struct RecordingController {
    phase: RecordingPhase,
}

impl RecordingController {
    pub fn phase(&self) -> RecordingPhase {
        self.phase
    }

    pub fn start_recording(&mut self) -> RecordingPhase {
        match self.phase {
            RecordingPhase::Idle => {
                self.phase = RecordingPhase::Recording;
                start_recording();
            }
            RecordingPhase::Recording => {
                info!("Start recording requested while already recording");
            }
            RecordingPhase::Processing => {
                info!("Start recording requested while processing audio");
            }
        }

        self.phase
    }

    pub fn stop_recording(&mut self) -> RecordingPhase {
        match self.phase {
            RecordingPhase::Idle => {
                info!("Stop recording requested while not recording");
            }
            RecordingPhase::Recording => {
                stop_recording();
                self.phase = RecordingPhase::Processing;
            }
            RecordingPhase::Processing => {
                info!("Stop recording requested while processing audio");
            }
        }

        self.phase
    }

    pub fn finish_processing(&mut self) -> RecordingPhase {
        if self.phase == RecordingPhase::Processing {
            transcribe_audio();
            self.phase = RecordingPhase::Idle;
        }

        self.phase
    }
}

pub fn start_recording() {
    info!("Start recording");
}

pub fn stop_recording() {
    info!("Stop recording");
}

pub fn transcribe_audio() {
    info!("Transcribe audio placeholder");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_start_and_stop_are_safe() {
        let mut controller = RecordingController::default();

        assert_eq!(controller.phase(), RecordingPhase::Idle);
        assert_eq!(controller.start_recording(), RecordingPhase::Recording);
        assert_eq!(controller.start_recording(), RecordingPhase::Recording);

        assert_eq!(controller.stop_recording(), RecordingPhase::Processing);
        assert_eq!(controller.stop_recording(), RecordingPhase::Processing);
        assert_eq!(controller.finish_processing(), RecordingPhase::Idle);
        assert_eq!(controller.stop_recording(), RecordingPhase::Idle);
    }

    #[test]
    fn processing_can_only_finish_from_processing_phase() {
        let mut controller = RecordingController::default();

        assert_eq!(controller.finish_processing(), RecordingPhase::Idle);
        assert_eq!(controller.start_recording(), RecordingPhase::Recording);
        assert_eq!(controller.stop_recording(), RecordingPhase::Processing);
        assert_eq!(controller.finish_processing(), RecordingPhase::Idle);
    }
}
