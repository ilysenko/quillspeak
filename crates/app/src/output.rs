use shared::OutputAction;
use tracing::info;

use crate::transcription::TranscriptionResult;

pub struct OutputService;

impl OutputService {
    pub fn apply(shortcut_id: &str, result: &TranscriptionResult) {
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
