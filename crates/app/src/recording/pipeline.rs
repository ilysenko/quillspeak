use anyhow::{Result, ensure};

use crate::audio::AudioCaptureService;
use crate::transcription::{TranscriptionPlan, TranscriptionRequest};

#[derive(Default)]
pub struct RecordingPipeline {
    capture: Option<AudioCaptureService>,
    plan: Option<TranscriptionPlan>,
}

impl RecordingPipeline {
    pub fn start(&mut self, plan: TranscriptionPlan) -> Result<()> {
        ensure!(
            self.capture.is_none(),
            "audio capture is already active for another recording"
        );
        let capture = AudioCaptureService::start(&plan.input)?;
        self.capture = Some(capture);
        self.plan = Some(plan);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<TranscriptionRequest> {
        let capture = self
            .capture
            .take()
            .ok_or_else(|| anyhow::anyhow!("audio capture was not active"))?;
        let plan = self
            .plan
            .take()
            .ok_or_else(|| anyhow::anyhow!("transcription plan was not active"))?;
        Ok(plan.into_request(capture.stop()))
    }

    pub fn cancel(&mut self) {
        self.capture.take();
        self.plan.take();
    }
}
