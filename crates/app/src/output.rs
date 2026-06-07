use shared::OutputAction;
use tracing::info;

use crate::transcription::{TranscriptionResult, TranscriptionStatus};

pub struct OutputService;

impl OutputService {
    pub fn apply(shortcut_id: &str, result: &TranscriptionResult) {
        if let TranscriptionStatus::Skipped { reason } = result.status {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                reason = reason.label(),
                "Skipping output action because transcription was skipped"
            );
            return;
        }

        if result.text.trim().is_empty() {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                "Skipping output action because recognized text is empty"
            );
            return;
        }

        match &result.output {
            OutputAction::Clipboard => info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                text = %result.text,
                "Would copy recognized text to clipboard"
            ),
            OutputAction::Script { path } => info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                text = %result.text,
                script = path,
                "Would run output script with recognized text argument"
            ),
        }
    }
}
