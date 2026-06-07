use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use shared::ComputeBackend;
use tracing::{debug, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

use crate::audio::prepare_whisper_audio;
use crate::transcription::compute::{context_params, default_thread_count, whisper_language};
use crate::transcription::types::{
    TranscriptionDebugInfo, TranscriptionRequest, TranscriptionResult, TranscriptionSegment,
};

pub(super) struct WhisperEngine {
    cached_context: SingleModelCache<WhisperContext>,
    keep_model_loaded: bool,
}

impl WhisperEngine {
    pub(super) fn new(keep_model_loaded: bool) -> Self {
        Self {
            cached_context: SingleModelCache::default(),
            keep_model_loaded,
        }
    }

    pub(super) fn set_keep_model_loaded(&mut self, keep_model_loaded: bool) {
        if self.keep_model_loaded == keep_model_loaded {
            return;
        }

        self.keep_model_loaded = keep_model_loaded;
        if keep_model_loaded {
            info!("whisper model cache enabled");
        } else {
            self.clear_cached_context("whisper model cache disabled");
        }
    }

    pub(super) fn clear_cached_model_path(&mut self, model_path: &Path) {
        let Some(cached) = self.cached_context.take_if_path(model_path) else {
            return;
        };
        info!(
            model_path = %cached.key.model_path.display(),
            compute_backend = %cached.key.compute_backend.as_str(),
            "cleared cached whisper model for deleted model"
        );
    }

    pub(super) fn transcribe(
        &mut self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResult> {
        if request.audio.samples.is_empty() {
            anyhow::bail!("recorded audio is empty");
        }

        let prepared = prepare_whisper_audio(&request.audio);
        if prepared.samples.is_empty() {
            anyhow::bail!("prepared whisper audio is empty");
        }

        let key = ModelCacheKey::from_request(&request);
        if self.keep_model_loaded {
            let context = self.cached_context(&request, key)?;
            Self::run_transcription(context, request, prepared)
        } else {
            let model_id = request.model_id.clone();
            let model_path = request.model_path.clone();
            let compute_backend = request.compute_backend;
            let context = Self::load_context(&request)?;
            let result = Self::run_transcription(&context, request, prepared);
            info!(
                model_id = %model_id,
                model_path = %model_path.display(),
                compute_backend = %compute_backend.as_str(),
                "dropping whisper model after transcription"
            );
            result
        }
    }

    fn cached_context(
        &mut self,
        request: &TranscriptionRequest,
        key: ModelCacheKey,
    ) -> Result<&WhisperContext> {
        if self.cached_context.get(&key).is_some() {
            info!(
                shortcut_id = %request.shortcut_id,
                model_id = %request.model_id,
                model_path = %request.model_path.display(),
                compute_backend = %request.compute_backend.as_str(),
                "using cached whisper model"
            );
            return Ok(self
                .cached_context
                .get(&key)
                .expect("cached key was checked before returning"));
        }

        if let Some(cached) = self.cached_context.clear() {
            info!(
                model_path = %cached.key.model_path.display(),
                compute_backend = %cached.key.compute_backend.as_str(),
                "evicted previous cached whisper model"
            );
        }

        let context = Self::load_context(request)?;
        self.cached_context.replace(key.clone(), context);
        Ok(self
            .cached_context
            .get(&key)
            .expect("cached context was inserted before returning"))
    }

    fn load_context(request: &TranscriptionRequest) -> Result<WhisperContext> {
        let params = context_params(request.compute_backend)?;
        info!(
            shortcut_id = %request.shortcut_id,
            model_id = %request.model_id,
            model_path = %request.model_path.display(),
            compute_backend = %request.compute_backend.as_str(),
            "loading whisper model"
        );
        let load_started = Instant::now();
        let context = WhisperContext::new_with_params(&request.model_path, params)
            .with_context(|| format!("failed to load model {}", request.model_path.display()))?;
        info!(
            shortcut_id = %request.shortcut_id,
            model_id = %request.model_id,
            model_path = %request.model_path.display(),
            compute_backend = %request.compute_backend.as_str(),
            load_duration_ms = load_started.elapsed().as_millis(),
            "loaded whisper model"
        );
        Ok(context)
    }

    fn run_transcription(
        context: &WhisperContext,
        request: TranscriptionRequest,
        prepared: crate::audio::PreparedAudio,
    ) -> Result<TranscriptionResult> {
        let mut state = context
            .create_state()
            .context("failed to create whisper state")?;
        let language = whisper_language(&request.language);
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language);
        params.set_detect_language(language.is_none());
        params.set_no_context(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_n_threads(default_thread_count());
        install_progress_logger(&mut params, &request);

        let audio_stats = request.audio.signal_stats();
        if audio_stats.is_near_silent() {
            warn!(
                shortcut_id = %request.shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms = request.audio.duration_ms(),
                source_frames = request.audio.frame_count(),
                audio_rms = audio_stats.rms,
                audio_peak = audio_stats.peak,
                "captured audio is near silent"
            );
        }
        info!(
            shortcut_id = %request.shortcut_id,
            shortcut_name = %request.shortcut_name,
            model_id = %request.model_id,
            input = %request.audio.input_label,
            capture_duration_ms = request.audio.duration_ms(),
            source_sample_rate = request.audio.sample_rate,
            source_channels = request.audio.channels,
            source_frames = request.audio.frame_count(),
            dropped_samples = request.audio.dropped_samples,
            missed_audio_chunks = request.audio.missed_chunks,
            audio_rms = audio_stats.rms,
            audio_peak = audio_stats.peak,
            whisper_samples = prepared.samples.len(),
            "captured audio prepared for whisper"
        );
        debug!(
            shortcut_id = %request.shortcut_id,
            shortcut_name = %request.shortcut_name,
            model_id = %request.model_id,
            model_path = %request.model_path.display(),
            language = %request.language,
            compute_backend = %request.compute_backend.as_str(),
            output = request.output.label(),
            input = %request.audio.input_label,
            source_sample_rate = request.audio.sample_rate,
            source_channels = request.audio.channels,
            source_frames = request.audio.frame_count(),
            dropped_samples = request.audio.dropped_samples,
            missed_audio_chunks = request.audio.missed_chunks,
            whisper_samples = prepared.samples.len(),
            "starting whisper transcription"
        );

        let inference_start = Instant::now();
        state
            .full(params, &prepared.samples)
            .context("whisper inference failed")?;
        let inference_duration_ms = inference_start.elapsed().as_millis();
        info!(
            shortcut_id = %request.shortcut_id,
            model_id = %request.model_id,
            inference_duration_ms,
            "finished whisper inference"
        );

        let mut segments = Vec::new();
        for index in 0..state.full_n_segments() {
            let Some(segment) = state.get_segment(index) else {
                continue;
            };
            segments.push(TranscriptionSegment {
                start_centiseconds: segment.start_timestamp(),
                end_centiseconds: segment.end_timestamp(),
                text: segment.to_str_lossy()?.into_owned(),
            });
        }

        let text = segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let capture_duration_ms = request.audio.duration_ms();
        let source_frames = request.audio.frame_count();
        let dropped_samples = request.audio.dropped_samples;
        let missed_audio_chunks = request.audio.missed_chunks;

        let result = TranscriptionResult {
            text,
            segments,
            output: request.output.clone(),
            debug: TranscriptionDebugInfo {
                shortcut_name: request.shortcut_name,
                model_id: request.model_id,
                model_path: request.model_path,
                language: request.language,
                compute_backend: request.compute_backend,
                output_label: request.output.label().to_string(),
                capture_duration_ms,
                input_label: request.audio.input_label,
                source_sample_rate: prepared.source_sample_rate,
                source_channels: prepared.source_channels,
                source_frames,
                dropped_samples,
                missed_audio_chunks,
                audio_rms: audio_stats.rms,
                audio_peak: audio_stats.peak,
                whisper_sample_rate: prepared.sample_rate,
                whisper_samples: prepared.samples.len(),
                inference_duration_ms,
            },
        };

        info!(
            shortcut_id = %request.shortcut_id,
            model_id = %result.debug.model_id,
            language = %result.debug.language,
            text = %result.text,
            "recognized text"
        );
        if result.text.is_empty() {
            warn!(
                shortcut_id = %request.shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                segment_count = result.segments.len(),
                audio_rms = result.debug.audio_rms,
                audio_peak = result.debug.audio_peak,
                capture_duration_ms = result.debug.capture_duration_ms,
                inference_duration_ms = result.debug.inference_duration_ms,
                "recognized text is empty"
            );
        }
        debug!(?result, "transcription debug result");
        Ok(result)
    }

    fn clear_cached_context(&mut self, reason: &str) {
        let Some(cached) = self.cached_context.clear() else {
            return;
        };
        info!(
            model_path = %cached.key.model_path.display(),
            compute_backend = %cached.key.compute_backend.as_str(),
            reason,
            "cleared cached whisper model"
        );
    }
}

fn install_progress_logger(params: &mut FullParams<'_, '_>, request: &TranscriptionRequest) {
    let shortcut_id = request.shortcut_id.clone();
    let model_id = request.model_id.clone();
    let mut last_logged = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .unwrap_or_else(Instant::now);
    let mut last_progress = -1;

    params.set_progress_callback_safe(move |progress| {
        if progress < 100
            && progress - last_progress < 10
            && last_logged.elapsed() < Duration::from_secs(5)
        {
            return;
        }

        last_progress = progress;
        last_logged = Instant::now();
        info!(
            shortcut_id = %shortcut_id,
            model_id = %model_id,
            progress_percent = progress,
            "whisper transcription progress"
        );
    });
}

impl Default for WhisperEngine {
    fn default() -> Self {
        Self::new(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ModelCacheKey {
    model_path: PathBuf,
    compute_backend: ComputeBackend,
}

impl ModelCacheKey {
    fn from_request(request: &TranscriptionRequest) -> Self {
        Self {
            model_path: request.model_path.clone(),
            compute_backend: request.compute_backend,
        }
    }
}

struct CachedModel<T> {
    key: ModelCacheKey,
    value: T,
}

struct SingleModelCache<T> {
    entry: Option<CachedModel<T>>,
}

impl<T> SingleModelCache<T> {
    fn get(&self, key: &ModelCacheKey) -> Option<&T> {
        self.entry
            .as_ref()
            .filter(|entry| entry.key == *key)
            .map(|entry| &entry.value)
    }

    fn replace(&mut self, key: ModelCacheKey, value: T) -> Option<CachedModel<T>> {
        self.entry.replace(CachedModel { key, value })
    }

    fn take_if_path(&mut self, model_path: &Path) -> Option<CachedModel<T>> {
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.key.model_path == model_path)
        {
            self.entry.take()
        } else {
            None
        }
    }

    fn clear(&mut self) -> Option<CachedModel<T>> {
        self.entry.take()
    }
}

impl<T> Default for SingleModelCache<T> {
    fn default() -> Self {
        Self { entry: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_key(path: &str, compute_backend: ComputeBackend) -> ModelCacheKey {
        ModelCacheKey {
            model_path: PathBuf::from(path),
            compute_backend,
        }
    }

    #[test]
    fn single_model_cache_keeps_only_latest_context() {
        let mut cache = SingleModelCache::default();
        let first = cache_key("/tmp/first.bin", ComputeBackend::Cpu);
        let second = cache_key("/tmp/second.bin", ComputeBackend::Cpu);

        assert!(cache.replace(first.clone(), "first").is_none());
        assert_eq!(cache.get(&first), Some(&"first"));

        let evicted = cache.replace(second.clone(), "second").expect("old entry");
        assert_eq!(evicted.key, first);
        assert_eq!(cache.get(&second), Some(&"second"));
    }

    #[test]
    fn single_model_cache_takes_matching_model_path() {
        let mut cache = SingleModelCache::default();
        let key = cache_key("/tmp/model.bin", ComputeBackend::Auto);
        cache.replace(key.clone(), "context");

        assert!(cache.take_if_path(Path::new("/tmp/other.bin")).is_none());
        assert_eq!(
            cache
                .take_if_path(Path::new("/tmp/model.bin"))
                .map(|entry| entry.key),
            Some(key)
        );
        assert!(
            cache
                .get(&cache_key("/tmp/model.bin", ComputeBackend::Auto))
                .is_none()
        );
    }
}
