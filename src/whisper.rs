use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[allow(dead_code)]
pub trait WhisperRecognizer: Send + Sync {
    fn configure_model(
        &self,
        model: &str,
        backend: WhisperBackendPreference,
        gpu_device: i32,
    ) -> Result<()>;
    fn transcribe(&self, samples: &[i16]) -> Result<String>;
    fn runtime_status(&self) -> WhisperRuntimeStatus;
    fn available_backends(&self) -> Vec<WhisperBackend>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhisperBackendPreference {
    #[default]
    Auto,
    Cuda,
    Vulkan,
    Cpu,
}

impl fmt::Display for WhisperBackendPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Cuda => write!(f, "cuda"),
            Self::Vulkan => write!(f, "vulkan"),
            Self::Cpu => write!(f, "cpu"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperBackend {
    Cuda,
    Vulkan,
    Cpu,
}

impl WhisperBackend {
    fn is_gpu(self) -> bool {
        matches!(self, Self::Cuda | Self::Vulkan)
    }

    fn is_compiled(self) -> bool {
        match self {
            Self::Cuda => cfg!(feature = "gpu-cuda"),
            Self::Vulkan => cfg!(feature = "gpu-vulkan"),
            Self::Cpu => true,
        }
    }
}

impl fmt::Display for WhisperBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cuda => write!(f, "CUDA GPU"),
            Self::Vulkan => write!(f, "Vulkan GPU"),
            Self::Cpu => write!(f, "CPU"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperRuntimeStatus {
    pub requested_backend: WhisperBackendPreference,
    pub active_backend: Option<WhisperBackend>,
    pub message: String,
}

impl Default for WhisperRuntimeStatus {
    fn default() -> Self {
        Self {
            requested_backend: WhisperBackendPreference::Auto,
            active_backend: None,
            message: "No Whisper model selected.".to_string(),
        }
    }
}

impl WhisperRuntimeStatus {
    pub fn summary(&self) -> String {
        match self.active_backend {
            Some(active_backend) if self.requested_backend == WhisperBackendPreference::Auto => {
                format!("Auto: {active_backend}")
            }
            Some(active_backend) => {
                format!("{}: {active_backend}", self.requested_backend)
            }
            None => self.message.clone(),
        }
    }
}

struct LoadedWhisperModel {
    backend: WhisperBackend,
    context: WhisperContext,
}

pub struct RuntimeWhisperRecognizer {
    loaded_model: Mutex<Option<Arc<LoadedWhisperModel>>>,
    status: Mutex<WhisperRuntimeStatus>,
}

impl Default for RuntimeWhisperRecognizer {
    fn default() -> Self {
        Self {
            loaded_model: Mutex::new(None),
            status: Mutex::new(WhisperRuntimeStatus::default()),
        }
    }
}

impl RuntimeWhisperRecognizer {
    fn set_status(&self, status: WhisperRuntimeStatus) {
        *self.status.lock().expect("Whisper status was poisoned") = status;
    }

    fn load_context(
        model_path: &str,
        backend: WhisperBackend,
        gpu_device: i32,
    ) -> Result<WhisperContext> {
        let mut params = WhisperContextParameters::default();

        if backend.is_gpu() {
            params.use_gpu(true);
            params.gpu_device(gpu_device);
        } else {
            params.use_gpu(false);
        }

        WhisperContext::new_with_params(model_path, params)
            .map_err(|error| anyhow!("failed to load model with {backend}: {error}"))
    }
}

impl WhisperRecognizer for RuntimeWhisperRecognizer {
    fn configure_model(
        &self,
        model: &str,
        backend: WhisperBackendPreference,
        gpu_device: i32,
    ) -> Result<()> {
        let model_path = model.trim();
        if model_path.is_empty() {
            self.loaded_model
                .lock()
                .expect("Whisper model state was poisoned")
                .take();
            self.set_status(WhisperRuntimeStatus {
                requested_backend: backend,
                active_backend: None,
                message: "No Whisper model selected.".to_string(),
            });
            return Ok(());
        }

        if !Path::new(model_path).exists() {
            self.loaded_model
                .lock()
                .expect("Whisper model state was poisoned")
                .take();
            self.set_status(WhisperRuntimeStatus {
                requested_backend: backend,
                active_backend: None,
                message: format!("Model file does not exist: {model_path}"),
            });
            bail!("model file does not exist: {model_path}");
        }

        let mut failures = Vec::new();
        for candidate in candidate_backends(backend) {
            if !candidate.is_compiled() {
                failures.push(format!(
                    "{candidate} support is not compiled into this binary"
                ));
                continue;
            }

            match Self::load_context(model_path, candidate, gpu_device) {
                Ok(context) => {
                    self.loaded_model
                        .lock()
                        .expect("Whisper model state was poisoned")
                        .replace(Arc::new(LoadedWhisperModel {
                            backend: candidate,
                            context,
                        }));
                    self.set_status(WhisperRuntimeStatus {
                        requested_backend: backend,
                        active_backend: Some(candidate),
                        message: if failures.is_empty() {
                            format!("Whisper model loaded on {candidate}.")
                        } else {
                            format!(
                                "Whisper model loaded on {candidate}; fallback notes: {}",
                                failures.join("; ")
                            )
                        },
                    });
                    return Ok(());
                }
                Err(error) => failures.push(error.to_string()),
            }
        }

        self.loaded_model
            .lock()
            .expect("Whisper model state was poisoned")
            .take();
        let message = format!("Unable to load Whisper model. {}", failures.join("; "));
        self.set_status(WhisperRuntimeStatus {
            requested_backend: backend,
            active_backend: None,
            message: message.clone(),
        });
        bail!("{message}");
    }

    fn transcribe(&self, samples: &[i16]) -> Result<String> {
        let loaded_model = self
            .loaded_model
            .lock()
            .expect("Whisper model state was poisoned")
            .clone()
            .context("no Whisper model loaded")?;
        let mut state = loaded_model
            .context
            .create_state()
            .context("failed to create Whisper state")?;
        let samples = samples
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("auto"));
        // `detect_language` makes whisper.cpp stop after language detection on
        // some builds. `language = auto` keeps autodetect while still decoding.
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_single_segment(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let _ = state
            .full(params, &samples)
            .with_context(|| format!("Whisper transcription failed on {}", loaded_model.backend))?;

        let segment_count = state.full_n_segments()?;
        let debug_enabled = voice_debug_enabled();
        eprintln!("Whisper segments: {segment_count}");
        if debug_enabled {
            eprintln!("Whisper produced {segment_count} segment(s).");
        }
        let mut text = String::new();
        for segment in 0..segment_count {
            let segment_text = state.full_get_segment_text_lossy(segment)?;
            let segment_text = segment_text.trim();
            eprintln!(
                "Whisper segment {segment}: {}",
                display_transcription_text(segment_text)
            );
            text.push_str(segment_text);
            text.push(' ');
        }

        let text = text.trim().to_string();
        eprintln!("Whisper final text: {}", display_transcription_text(&text));
        if debug_enabled {
            if text.is_empty() {
                eprintln!("Whisper returned empty text.");
            } else {
                eprintln!("Whisper final text ({} chars).", text.chars().count());
            }
        }

        Ok(text)
    }

    fn runtime_status(&self) -> WhisperRuntimeStatus {
        self.status
            .lock()
            .expect("Whisper status was poisoned")
            .clone()
    }

    fn available_backends(&self) -> Vec<WhisperBackend> {
        available_backends()
    }
}

#[derive(Debug, Default)]
pub struct StubWhisperRecognizer {
    configured_model: Mutex<String>,
}

impl StubWhisperRecognizer {
    #[cfg(test)]
    pub fn configured_model(&self) -> String {
        self.configured_model
            .lock()
            .expect("stub Whisper state was poisoned")
            .clone()
    }
}

impl WhisperRecognizer for StubWhisperRecognizer {
    fn configure_model(
        &self,
        model: &str,
        _backend: WhisperBackendPreference,
        _gpu_device: i32,
    ) -> Result<()> {
        *self
            .configured_model
            .lock()
            .expect("stub Whisper state was poisoned") = model.to_string();
        Ok(())
    }

    fn transcribe(&self, _samples: &[i16]) -> Result<String> {
        Ok(String::new())
    }

    fn runtime_status(&self) -> WhisperRuntimeStatus {
        WhisperRuntimeStatus {
            requested_backend: WhisperBackendPreference::Auto,
            active_backend: None,
            message: "Stub Whisper recognizer.".to_string(),
        }
    }

    fn available_backends(&self) -> Vec<WhisperBackend> {
        vec![WhisperBackend::Cpu]
    }
}

fn candidate_backends(preference: WhisperBackendPreference) -> Vec<WhisperBackend> {
    match preference {
        WhisperBackendPreference::Auto => available_backends(),
        WhisperBackendPreference::Cuda => vec![WhisperBackend::Cuda, WhisperBackend::Cpu],
        WhisperBackendPreference::Vulkan => vec![WhisperBackend::Vulkan, WhisperBackend::Cpu],
        WhisperBackendPreference::Cpu => vec![WhisperBackend::Cpu],
    }
}

fn available_backends() -> Vec<WhisperBackend> {
    let mut backends = Vec::new();

    if cfg!(feature = "gpu-cuda") {
        backends.push(WhisperBackend::Cuda);
    }
    if cfg!(feature = "gpu-vulkan") {
        backends.push(WhisperBackend::Vulkan);
    }
    backends.push(WhisperBackend::Cpu);

    backends
}

fn voice_debug_enabled() -> bool {
    std::env::var_os("VOICE_DEBUG").is_some()
}

fn display_transcription_text(text: &str) -> &str {
    if text.is_empty() { "<empty>" } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_tracks_configured_model() {
        let recognizer = StubWhisperRecognizer::default();

        recognizer
            .configure_model("base.en", WhisperBackendPreference::Auto, 0)
            .unwrap();

        assert_eq!(recognizer.configured_model(), "base.en");
    }

    #[test]
    fn auto_candidates_always_end_with_cpu() {
        let candidates = candidate_backends(WhisperBackendPreference::Auto);

        assert_eq!(candidates.last(), Some(&WhisperBackend::Cpu));
    }

    #[test]
    fn empty_model_clears_runtime_recognizer() {
        let recognizer = RuntimeWhisperRecognizer::default();

        recognizer
            .configure_model("", WhisperBackendPreference::Auto, 0)
            .unwrap();

        let status = recognizer.runtime_status();
        assert_eq!(status.active_backend, None);
        assert_eq!(status.summary(), "No Whisper model selected.");
    }
}
