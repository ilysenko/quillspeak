use tracing::info;

#[derive(Debug, Default)]
pub struct RecordingController {
    is_recording: bool,
}

impl RecordingController {
    pub fn start_recording(&mut self) {
        if self.is_recording {
            info!("Start recording requested while already recording");
            return;
        }

        self.is_recording = true;
        start_recording();
    }

    pub fn stop_recording(&mut self) {
        if !self.is_recording {
            info!("Stop recording requested while not recording");
            return;
        }

        self.is_recording = false;
        stop_recording();
        transcribe_audio();
    }

    #[cfg(test)]
    pub fn is_recording(&self) -> bool {
        self.is_recording
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

        controller.start_recording();
        controller.start_recording();
        assert!(controller.is_recording());

        controller.stop_recording();
        controller.stop_recording();
        assert!(!controller.is_recording());
    }
}
