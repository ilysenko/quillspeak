use std::time::Instant;

use anyhow::{Context, Result};
use shared::ComputeBackend;
use tracing::{info, warn};
use whisper_rs::WhisperContext;

use crate::transcription::compute::{CompiledWhisperBackends, context_params, cpu_context_params};
use crate::transcription::status::WhisperRuntimeStatus;
use crate::transcription::types::TranscriptionRequest;

pub(super) struct LoadedWhisperContext {
    pub(super) context: WhisperContext,
    pub(super) runtime_status: WhisperRuntimeStatus,
}

pub(super) fn load_context(request: &TranscriptionRequest) -> Result<LoadedWhisperContext> {
    let compiled_backends = CompiledWhisperBackends::current();
    if request.compute_backend == ComputeBackend::Auto && compiled_backends.has_gpu() {
        let params = context_params(request.compute_backend)?;
        match load_context_with_params(request, params, "auto-gpu") {
            Ok(context) => return Ok(context),
            Err(error) => {
                let gpu_error = format!("{error:#}");
                warn!(
                    shortcut_id = %request.shortcut_id,
                    model_id = %request.model_id,
                    model_path = %request.model_path.display(),
                    compiled_whisper_backends = %compiled_backends.display_label(),
                    error = %gpu_error,
                    "whisper auto GPU initialization failed; retrying with CPU"
                );
                return load_context_with_params(request, cpu_context_params(), "auto-cpu-fallback")
                    .with_context(|| {
                        format!(
                            "whisper auto GPU initialization failed ({gpu_error}); CPU fallback also failed"
                        )
                    });
            }
        }
    }

    let params = context_params(request.compute_backend)?;
    let effective_compute = if request.compute_backend == ComputeBackend::Auto {
        "auto-cpu"
    } else {
        request.compute_backend.as_str()
    };
    load_context_with_params(request, params, effective_compute)
}

fn load_context_with_params(
    request: &TranscriptionRequest,
    params: whisper_rs::WhisperContextParameters<'static>,
    effective_compute: &str,
) -> Result<LoadedWhisperContext> {
    let model_path = request.model_path.to_str().with_context(|| {
        format!(
            "model path is not valid UTF-8: {}",
            request.model_path.display()
        )
    })?;
    let whisper_gpu_requested = params.use_gpu;
    let whisper_flash_attention = params.flash_attn;
    let compiled_whisper_backends = CompiledWhisperBackends::current().display_label();
    info!(
        shortcut_id = %request.shortcut_id,
        model_id = %request.model_id,
        model_path = %request.model_path.display(),
        configured_compute = %request.compute_backend.as_str(),
        effective_compute,
        whisper_gpu_requested,
        whisper_flash_attention,
        compiled_whisper_backends = %compiled_whisper_backends,
        "loading whisper model"
    );
    let load_started = Instant::now();
    let context = WhisperContext::new_with_params(model_path, params)
        .with_context(|| format!("failed to load model {}", request.model_path.display()))?;
    info!(
        shortcut_id = %request.shortcut_id,
        model_id = %request.model_id,
        model_path = %request.model_path.display(),
        configured_compute = %request.compute_backend.as_str(),
        effective_compute,
        whisper_gpu_requested,
        whisper_flash_attention,
        compiled_whisper_backends = %compiled_whisper_backends,
        load_duration_ms = load_started.elapsed().as_millis(),
        "loaded whisper model"
    );
    Ok(LoadedWhisperContext {
        context,
        runtime_status: WhisperRuntimeStatus::loaded(
            request.compute_backend,
            request.model_id.clone(),
            effective_compute,
            whisper_gpu_requested,
        ),
    })
}
