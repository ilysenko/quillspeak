use anyhow::Result;
use shared::{AUTO_LANGUAGE_VALUE, ComputeBackend};
use tracing::warn;
use whisper_rs::WhisperContextParameters;

pub fn context_params(
    compute_backend: ComputeBackend,
) -> Result<WhisperContextParameters<'static>> {
    let mut params = WhisperContextParameters::new();
    match compute_backend {
        ComputeBackend::Auto => {
            params.use_gpu(true).flash_attn(true);
        }
        ComputeBackend::Cpu => {
            params.use_gpu(false);
        }
        ComputeBackend::Vulkan if cfg!(feature = "whisper-vulkan") => {
            params.use_gpu(true).flash_attn(true);
        }
        ComputeBackend::Cuda if cfg!(feature = "whisper-cuda") => {
            params.use_gpu(true).flash_attn(true);
        }
        ComputeBackend::Rocm if cfg!(feature = "whisper-rocm") => {
            params.use_gpu(true).flash_attn(true);
        }
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
        ComputeBackend::OpenVino => {
            anyhow::bail!("OpenVINO is not supported by the current whisper-rs integration");
        }
    }
    Ok(params)
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
        assert!(context_params(ComputeBackend::OpenVino).is_err());
    }
}
