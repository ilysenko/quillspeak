use anyhow::Result;
use shared::{AUTO_LANGUAGE_VALUE, ComputeBackend};
use tracing::{info, warn};
use whisper_rs::WhisperContextParameters;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledWhisperBackends {
    pub cuda: bool,
    pub rocm: bool,
    pub vulkan: bool,
}

impl CompiledWhisperBackends {
    pub const fn current() -> Self {
        Self {
            cuda: cfg!(feature = "whisper-cuda"),
            rocm: cfg!(feature = "whisper-rocm"),
            vulkan: cfg!(feature = "whisper-vulkan"),
        }
    }

    pub const fn has_gpu(self) -> bool {
        self.cuda || self.rocm || self.vulkan
    }

    pub fn display_label(self) -> String {
        let mut backends = vec!["cpu"];
        if self.vulkan {
            backends.push("vulkan");
        }
        if self.cuda {
            backends.push("cuda");
        }
        if self.rocm {
            backends.push("rocm");
        }
        backends.join(",")
    }
}

pub fn context_params(
    compute_backend: ComputeBackend,
) -> Result<WhisperContextParameters<'static>> {
    let compiled_backends = CompiledWhisperBackends::current();
    match compute_backend {
        ComputeBackend::Auto => {
            if compiled_backends.has_gpu() {
                Ok(gpu_context_params())
            } else {
                info!("whisper auto compute is CPU-only because no GPU backend is compiled");
                Ok(cpu_context_params())
            }
        }
        ComputeBackend::Cpu => Ok(cpu_context_params()),
        ComputeBackend::Vulkan if cfg!(feature = "whisper-vulkan") => Ok(gpu_context_params()),
        ComputeBackend::Cuda if cfg!(feature = "whisper-cuda") => Ok(gpu_context_params()),
        ComputeBackend::Rocm if cfg!(feature = "whisper-rocm") => Ok(gpu_context_params()),
        ComputeBackend::Vulkan | ComputeBackend::Cuda | ComputeBackend::Rocm => {
            warn!(
                compute_backend = %compute_backend.as_str(),
                "selected whisper compute backend is not enabled in this build"
            );
            anyhow::bail!(
                "whisper compute backend {} is not enabled in this build",
                compute_backend.as_str()
            );
        }
    }
}

pub fn cpu_context_params() -> WhisperContextParameters<'static> {
    let mut params = WhisperContextParameters::new();
    params.use_gpu(false);
    params
}

fn gpu_context_params() -> WhisperContextParameters<'static> {
    let mut params = WhisperContextParameters::new();
    params.use_gpu(true).flash_attn(false);
    params
}

pub fn whisper_language(language: &str) -> Option<&str> {
    if language == AUTO_LANGUAGE_VALUE {
        None
    } else {
        Some(language)
    }
}

pub fn default_thread_count() -> i32 {
    std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8) as i32)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_language_maps_to_whisper_auto_detection() {
        assert_eq!(whisper_language(AUTO_LANGUAGE_VALUE), None);
        assert_eq!(whisper_language("uk"), Some("uk"));
    }

    #[test]
    fn cpu_backend_is_supported_without_gpu_feature() {
        assert!(context_params(ComputeBackend::Cpu).is_ok());
    }

    #[test]
    fn auto_backend_is_supported_without_gpu_feature() {
        assert!(context_params(ComputeBackend::Auto).is_ok());
    }

    #[test]
    fn compiled_backends_match_enabled_features() {
        let backends = CompiledWhisperBackends::current();

        assert_eq!(backends.cuda, cfg!(feature = "whisper-cuda"));
        assert_eq!(backends.rocm, cfg!(feature = "whisper-rocm"));
        assert_eq!(backends.vulkan, cfg!(feature = "whisper-vulkan"));
        assert_eq!(
            backends.has_gpu(),
            backends.cuda || backends.rocm || backends.vulkan
        );
    }

    #[test]
    fn explicit_unsupported_gpu_backends_fail_without_features() {
        if !cfg!(feature = "whisper-vulkan") {
            assert!(context_params(ComputeBackend::Vulkan).is_err());
        }
        if !cfg!(feature = "whisper-cuda") {
            assert!(context_params(ComputeBackend::Cuda).is_err());
        }
        if !cfg!(feature = "whisper-rocm") {
            assert!(context_params(ComputeBackend::Rocm).is_err());
        }
    }
}
