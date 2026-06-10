use shared::ComputeBackend;
use std::ffi::CStr;
use std::sync::OnceLock;

use crate::transcription::compute::CompiledWhisperBackends;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperRuntimeStatus {
    pub configured_compute: ComputeBackend,
    pub compiled_backends: String,
    pub whisper_system_info: String,
    pub state: WhisperRuntimeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhisperRuntimeState {
    NotLoaded,
    Loaded {
        model_id: String,
        effective_compute: String,
        gpu_requested: bool,
    },
    Failed {
        error: String,
    },
}

impl WhisperRuntimeStatus {
    pub fn initial(configured_compute: ComputeBackend) -> Self {
        Self {
            configured_compute,
            compiled_backends: CompiledWhisperBackends::current().display_label(),
            whisper_system_info: whisper_system_info(),
            state: WhisperRuntimeState::NotLoaded,
        }
    }

    pub fn loaded(
        configured_compute: ComputeBackend,
        model_id: impl Into<String>,
        effective_compute: impl Into<String>,
        gpu_requested: bool,
    ) -> Self {
        Self {
            configured_compute,
            compiled_backends: CompiledWhisperBackends::current().display_label(),
            whisper_system_info: whisper_system_info(),
            state: WhisperRuntimeState::Loaded {
                model_id: model_id.into(),
                effective_compute: effective_compute.into(),
                gpu_requested,
            },
        }
    }

    pub fn failed(configured_compute: ComputeBackend, error: impl Into<String>) -> Self {
        Self {
            configured_compute,
            compiled_backends: CompiledWhisperBackends::current().display_label(),
            whisper_system_info: whisper_system_info(),
            state: WhisperRuntimeState::Failed {
                error: error.into(),
            },
        }
    }

    pub fn summary(&self) -> String {
        let state = match &self.state {
            WhisperRuntimeState::NotLoaded => "No model loaded yet".to_string(),
            WhisperRuntimeState::Loaded {
                model_id,
                effective_compute,
                gpu_requested,
            } if *gpu_requested => {
                format!("GPU via {effective_compute}; model {model_id}")
            }
            WhisperRuntimeState::Loaded {
                model_id,
                effective_compute,
                ..
            } if effective_compute == "auto-cpu-fallback" => {
                format!("CPU fallback after auto GPU failed; model {model_id}")
            }
            WhisperRuntimeState::Loaded {
                model_id,
                effective_compute,
                ..
            } => {
                format!("CPU via {effective_compute}; model {model_id}")
            }
            WhisperRuntimeState::Failed { error } => format!("Failed: {error}"),
        };

        format!(
            "{state}; configured {}; compiled {}; Whisper {}",
            self.configured_compute.as_str(),
            self.compiled_backends,
            self.whisper_system_info
        )
    }
}

fn whisper_system_info() -> String {
    // whisper_print_system_info writes into an unsynchronized static
    // std::string inside whisper.cpp; concurrent calls corrupt the heap.
    // OnceLock guarantees the FFI call happens exactly once per process.
    static SYSTEM_INFO: OnceLock<String> = OnceLock::new();
    SYSTEM_INFO
        .get_or_init(|| {
            let c_buffer = unsafe { whisper_rs_sys::whisper_print_system_info() };
            if c_buffer.is_null() {
                return "unavailable".to_string();
            }

            let system_info = unsafe { CStr::from_ptr(c_buffer) }
                .to_string_lossy()
                .trim()
                .to_string();
            if system_info.is_empty() {
                "unavailable".to_string()
            } else {
                system_info
            }
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_gpu_status_mentions_gpu_and_effective_compute() {
        let status = WhisperRuntimeStatus::loaded(ComputeBackend::Auto, "large", "auto-gpu", true);

        let summary = status.summary();
        assert!(summary.contains("GPU via auto-gpu"));
        assert!(summary.contains("configured auto"));
    }

    #[test]
    fn cpu_fallback_status_is_not_reported_as_gpu() {
        let status =
            WhisperRuntimeStatus::loaded(ComputeBackend::Auto, "large", "auto-cpu-fallback", false);

        let summary = status.summary();
        assert!(summary.contains("CPU fallback"));
        assert!(!summary.contains("GPU via"));
    }
}
