use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::settings::widgets::{advanced_hotkey_status, output_tools_status, property_row};
use crate::system_audio::speaker_mute_tools_status;
use crate::transcription::{WhisperRuntimeState, WhisperRuntimeStatus};

const STATUS_MAX_WIDTH: i32 = 740;
const STATUS_TIGHTENING_WIDTH: i32 = 600;

#[derive(Clone)]
pub struct StatusPage {
    page: adw::Clamp,
}

impl StatusPage {
    pub fn widget(&self) -> &adw::Clamp {
        &self.page
    }
}

pub fn build(whisper_runtime_status: WhisperRuntimeStatus) -> StatusPage {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(28)
        .margin_bottom(28)
        .build();
    let page = adw::Clamp::builder()
        .maximum_size(STATUS_MAX_WIDTH)
        .tightening_threshold(STATUS_TIGHTENING_WIDTH)
        .hexpand(true)
        .vexpand(true)
        .valign(gtk::Align::Start)
        .child(&content)
        .build();

    let app_group = adw::PreferencesGroup::builder()
        .title("Application")
        .description("Runtime readiness for shortcuts, clipboard transport, paste tools, and speaker mute support.")
        .build();
    app_group.add(&property_row("Advanced hotkeys", advanced_hotkey_status()));
    let output_tools = output_tools_status();
    app_group.add(&property_row("Output tools", &output_tools));
    let speaker_mute_tools = speaker_mute_tools_status();
    app_group.add(&property_row("Audio mute tools", &speaker_mute_tools));

    let whisper_group = adw::PreferencesGroup::builder()
        .title("Whisper compute")
        .description("What Whisper is configured to use and what backend the current process actually initialized.")
        .build();
    for (title, value) in whisper_status_rows(&whisper_runtime_status) {
        whisper_group.add(&property_row(title, &value));
    }

    let features_group = adw::PreferencesGroup::builder()
        .title("Whisper features")
        .description("Feature flags reported by whisper.cpp for this build and machine.")
        .build();
    for (title, value) in whisper_feature_rows(&whisper_runtime_status.whisper_system_info) {
        features_group.add(&property_row(&title, &value));
    }

    content.append(&app_group);
    content.append(&whisper_group);
    content.append(&features_group);
    StatusPage { page }
}

fn whisper_status_rows(status: &WhisperRuntimeStatus) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        (
            "Configured compute",
            status.configured_compute.as_str().to_string(),
        ),
        ("Compiled backends", status.compiled_backends.clone()),
    ];

    match &status.state {
        WhisperRuntimeState::NotLoaded => {
            rows.insert(0, ("Runtime state", "No model loaded yet".to_string()));
            rows.push(("Active model", "None".to_string()));
            rows.push((
                "Effective compute",
                "Unknown until the first transcription loads a model".to_string(),
            ));
            rows.push(("GPU usage", "Not active".to_string()));
        }
        WhisperRuntimeState::Loaded {
            model_id,
            effective_compute,
            gpu_requested,
        } => {
            rows.insert(0, ("Runtime state", "Loaded".to_string()));
            rows.push(("Active model", model_id.clone()));
            rows.push(("Effective compute", effective_compute.clone()));
            rows.push((
                "GPU usage",
                if *gpu_requested {
                    format!("Active via {effective_compute}")
                } else if effective_compute == "auto-cpu-fallback" {
                    "CPU fallback after auto GPU initialization failed".to_string()
                } else {
                    "CPU".to_string()
                },
            ));
        }
        WhisperRuntimeState::Failed { error } => {
            rows.insert(0, ("Runtime state", "Failed".to_string()));
            rows.push(("Active model", "None".to_string()));
            rows.push(("Effective compute", "Unavailable".to_string()));
            rows.push(("GPU usage", "Unavailable".to_string()));
            rows.push(("Last error", error.clone()));
        }
    }

    rows
}

fn whisper_feature_rows(system_info: &str) -> Vec<(String, String)> {
    let features = system_info
        .split('|')
        .filter_map(|part| {
            let mut pieces = part.trim().splitn(2, '=');
            let key = pieces.next()?.trim();
            let value = pieces.next()?.trim();
            (!key.is_empty()).then(|| (key.to_string(), feature_value(value)))
        })
        .collect::<Vec<_>>();

    if features.is_empty() {
        vec![("System info".to_string(), system_info.to_string())]
    } else {
        features
    }
}

fn feature_value(value: &str) -> String {
    match value {
        "1" => "yes".to_string(),
        "0" => "no".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_whisper_system_info_flags() {
        let rows = whisper_feature_rows("AVX = 1 | CUDA = 0 | BLAS = 1");

        assert_eq!(
            rows,
            vec![
                ("AVX".to_string(), "yes".to_string()),
                ("CUDA".to_string(), "no".to_string()),
                ("BLAS".to_string(), "yes".to_string()),
            ]
        );
    }
}
