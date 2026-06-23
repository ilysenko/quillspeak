use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Result, anyhow, ensure};
use shared::{AppConfig, ModelCatalogEntry, model_catalog_entry};

use crate::transcription::types::TranscriptionPlan;

pub fn build_transcription_plan(
    config: &AppConfig,
    ready_model_ids: &HashSet<String>,
    model_path: impl FnOnce(ModelCatalogEntry) -> PathBuf,
    recording_id: u64,
    shortcut_id: &str,
) -> Result<TranscriptionPlan> {
    let shortcut = config
        .shortcut_by_id(shortcut_id)
        .ok_or_else(|| anyhow!("unknown shortcut {shortcut_id}"))?;
    let model_id = shortcut.model_id.clone();
    let entry =
        model_catalog_entry(&model_id).ok_or_else(|| anyhow!("unknown model {model_id}"))?;
    ensure!(
        ready_model_ids.contains(&model_id),
        "model {model_id} is not downloaded; open Settings > Models and download it first"
    );
    let model_path = model_path(entry);
    ensure!(
        model_path.exists(),
        "model file is missing even though inventory marks it ready: {}",
        model_path.display()
    );

    Ok(TranscriptionPlan {
        recording_id,
        shortcut_id: shortcut.id.clone(),
        shortcut_name: shortcut.name.clone(),
        model_id,
        model_path,
        language: shortcut.language.clone(),
        compute_backend: config.general.compute_backend,
        mute_output_while_recording: shortcut.mute_output_while_recording,
        beep_on_recording: shortcut.beep_on_recording,
        beep_volume_percent: shortcut.beep_volume_percent,
        output: shortcut.output.clone(),
        input: config.general.audio_input.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use shared::{DEFAULT_MODEL_ID, DEFAULT_SHORTCUT_ID};

    use super::*;

    #[test]
    fn rejects_shortcut_when_model_is_not_ready() {
        let config = AppConfig::default();
        let ready_model_ids = HashSet::new();

        let result = build_transcription_plan(
            &config,
            &ready_model_ids,
            |entry| PathBuf::from(entry.filename),
            1,
            DEFAULT_SHORTCUT_ID,
        );

        assert!(result.is_err());
    }

    #[test]
    fn plan_snapshots_current_shortcut_settings() {
        let config = AppConfig::default();
        let ready_model_ids = HashSet::from([DEFAULT_MODEL_ID.to_string()]);
        let model_path = temp_model_path();
        fs::write(&model_path, b"model").expect("test model file should be writable");

        let plan = build_transcription_plan(
            &config,
            &ready_model_ids,
            |_| model_path.clone(),
            7,
            DEFAULT_SHORTCUT_ID,
        )
        .expect("ready model should build a plan");

        assert_eq!(plan.recording_id, 7);
        assert_eq!(plan.shortcut_id, DEFAULT_SHORTCUT_ID);
        assert_eq!(plan.model_id, DEFAULT_MODEL_ID);
        assert_eq!(plan.model_path, model_path);
        assert_eq!(plan.input, config.general.audio_input);
        assert!(!plan.mute_output_while_recording);
        assert!(!plan.beep_on_recording);
        assert_eq!(
            plan.beep_volume_percent,
            config.default_shortcut().beep_volume_percent
        );

        let _ = fs::remove_file(plan.model_path);
    }

    #[test]
    fn plan_snapshots_shortcut_mute_output() {
        let mut config = AppConfig::default();
        config.shortcuts[0].mute_output_while_recording = true;
        let ready_model_ids = HashSet::from([DEFAULT_MODEL_ID.to_string()]);
        let model_path = temp_model_path();
        fs::write(&model_path, b"model").expect("test model file should be writable");

        let plan = build_transcription_plan(
            &config,
            &ready_model_ids,
            |_| model_path.clone(),
            8,
            DEFAULT_SHORTCUT_ID,
        )
        .expect("ready model should build a plan");

        assert!(plan.mute_output_while_recording);
        let _ = fs::remove_file(plan.model_path);
    }

    #[test]
    fn plan_snapshots_shortcut_beep_setting() {
        let mut config = AppConfig::default();
        config.shortcuts[0].beep_on_recording = true;
        config.shortcuts[0].beep_volume_percent = 35;
        let ready_model_ids = HashSet::from([DEFAULT_MODEL_ID.to_string()]);
        let model_path = temp_model_path();
        fs::write(&model_path, b"model").expect("test model file should be writable");

        let plan = build_transcription_plan(
            &config,
            &ready_model_ids,
            |_| model_path.clone(),
            9,
            DEFAULT_SHORTCUT_ID,
        )
        .expect("ready model should build a plan");

        assert!(plan.beep_on_recording);
        assert_eq!(plan.beep_volume_percent, 35);
        let _ = fs::remove_file(plan.model_path);
    }

    fn temp_model_path() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("quillspeak-test-model-{suffix}.bin"))
    }
}
