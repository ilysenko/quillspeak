use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

use crate::audio::{PreparedAudio, prepare_whisper_audio};
use crate::transcription::cache::{ModelCacheKey, SingleModelCache};
use crate::transcription::compute::{default_thread_count, whisper_language};
use crate::transcription::debug_audio::maybe_write_debug_audio;
use crate::transcription::params::{LoadedWhisperContext, load_context};
use crate::transcription::skip::{
    pad_short_whisper_audio, skip_transcription_reason, skipped_transcription_result,
};
use crate::transcription::status::WhisperRuntimeStatus;
use crate::transcription::types::{
    TranscriptionDebugInfo, TranscriptionRequest, TranscriptionResult, TranscriptionSegment,
    TranscriptionStatus,
};

pub(super) struct WhisperEngine {
    cached_context: SingleModelCache<LoadedWhisperContext>,
    keep_model_loaded: bool,
    pending_runtime_status: Option<WhisperRuntimeStatus>,
}

impl WhisperEngine {
    pub(super) fn new(keep_model_loaded: bool) -> Self {
        Self {
            cached_context: SingleModelCache::default(),
            keep_model_loaded,
            pending_runtime_status: None,
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

        let mut prepared = prepare_whisper_audio(&request.audio)
            .context("failed to prepare captured audio for whisper")?;
        if prepared.samples.is_empty() {
            anyhow::bail!("prepared whisper audio is empty");
        }

        if let Some(skip_reason) = skip_transcription_reason(&request, &prepared) {
            let audio_stats = request.audio.signal_stats();
            warn!(
                shortcut_id = %request.shortcut_id,
                model_id = %request.model_id,
                input = %request.audio.input_label,
                reason = skip_reason.label(),
                capture_duration_ms = request.audio.duration_ms(),
                capture_wall_duration_ms = request.audio.wall_duration_ms(),
                startup_latency_ms = request.audio.startup_latency_ms,
                first_callback_latency_ms = request.audio.first_callback_latency_ms,
                audio_callback_count = request.audio.audio_callback_count,
                stale_callback_count = request.audio.stale_callback_count,
                stale_samples = request.audio.stale_samples,
                source_sample_rate = request.audio.sample_rate,
                source_channels = request.audio.channels,
                source_frames = request.audio.frame_count(),
                audio_rms = audio_stats.rms,
                audio_peak = audio_stats.peak,
                prepared_duration_ms = prepared.duration_ms(),
                whisper_samples = prepared.samples.len(),
                "skipping whisper transcription for unusable audio capture"
            );
            if let Err(error) = maybe_write_debug_audio(&request, &prepared) {
                warn!(?error, "failed to write skipped debug audio files");
            }
            self.clear_cached_context("unusable audio capture skipped");
            return Ok(skipped_transcription_result(
                request,
                prepared,
                audio_stats,
                skip_reason,
            ));
        }

        let unpadded_whisper_samples = prepared.samples.len();
        pad_short_whisper_audio(&mut prepared);
        if prepared.samples.len() != unpadded_whisper_samples {
            info!(
                shortcut_id = %request.shortcut_id,
                model_id = %request.model_id,
                unpadded_whisper_samples,
                padded_whisper_samples = prepared.samples.len(),
                sample_rate = prepared.sample_rate,
                "padded short whisper audio with trailing silence"
            );
        }

        let key = ModelCacheKey::from_request(&request);
        if self.keep_model_loaded {
            let result = {
                let context = self.cached_context(&request, key)?;
                Self::run_transcription(context, request, prepared)
            };
            self.handle_cache_after_transcription(&result);
            result
        } else {
            let model_id = request.model_id.clone();
            let model_path = request.model_path.clone();
            let compute_backend = request.compute_backend;
            let loaded = load_context(&request)?;
            self.pending_runtime_status = Some(loaded.runtime_status.clone());
            let result = Self::run_transcription(&loaded.context, request, prepared);
            info!(
                model_id = %model_id,
                model_path = %model_path.display(),
                compute_backend = %compute_backend.as_str(),
                "dropping whisper model after transcription"
            );
            result
        }
    }

    pub(super) fn clear_cached_context_for_config_change(&mut self, reason: &str) {
        self.clear_cached_context(reason);
    }

    pub(super) fn take_runtime_status_update(&mut self) -> Option<WhisperRuntimeStatus> {
        self.pending_runtime_status.take()
    }

    fn cached_context(
        &mut self,
        request: &TranscriptionRequest,
        key: ModelCacheKey,
    ) -> Result<&WhisperContext> {
        if let Some(cached) = self.cached_context.get(&key) {
            info!(
                shortcut_id = %request.shortcut_id,
                model_id = %request.model_id,
                model_path = %request.model_path.display(),
                compute_backend = %request.compute_backend.as_str(),
                "using cached whisper model"
            );
            self.pending_runtime_status = Some(cached.runtime_status.clone());
            return Ok(self
                .cached_context
                .get(&key)
                .map(|cached| &cached.context)
                .expect("cached key was checked before returning"));
        }

        if let Some(cached) = self.cached_context.clear() {
            info!(
                model_path = %cached.key.model_path.display(),
                compute_backend = %cached.key.compute_backend.as_str(),
                "evicted previous cached whisper model"
            );
        }

        let context = load_context(request)?;
        self.pending_runtime_status = Some(context.runtime_status.clone());
        self.cached_context.replace(key.clone(), context);
        Ok(self
            .cached_context
            .get(&key)
            .map(|cached| &cached.context)
            .expect("cached context was inserted before returning"))
    }

    fn run_transcription(
        context: &WhisperContext,
        request: TranscriptionRequest,
        prepared: PreparedAudio,
    ) -> Result<TranscriptionResult> {
        let mut state = context
            .create_state()
            .context("failed to create whisper state")?;
        let language = whisper_language(&request.language);
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language);
        // `detect_language=true` is a language-detection-only mode in whisper.cpp:
        // it returns after detection instead of continuing into transcription.
        params.set_detect_language(false);
        params.set_no_context(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_timestamps(false);
        params.set_single_segment(false);
        params.set_translate(false);
        params.set_n_threads(default_thread_count());

        let audio_stats = request.audio.signal_stats();
        let capture_duration_ms = request.audio.duration_ms();
        let capture_wall_duration_ms = request.audio.wall_duration_ms();
        let prepared_duration_ms = prepared.duration_ms();
        if audio_stats.is_near_silent() {
            warn!(
                shortcut_id = %request.shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms,
                capture_wall_duration_ms,
                source_frames = request.audio.frame_count(),
                audio_rms = audio_stats.rms,
                audio_peak = audio_stats.peak,
                "captured audio is near silent"
            );
        }
        if prepared_duration_ms < 800 {
            warn!(
                shortcut_id = %request.shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms,
                capture_wall_duration_ms,
                prepared_duration_ms,
                whisper_samples = prepared.samples.len(),
                "prepared whisper audio is very short; transcription may be empty"
            );
        }
        if request.audio.audio_callback_count < 2 {
            warn!(
                shortcut_id = %request.shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms,
                capture_wall_duration_ms,
                audio_callback_count = request.audio.audio_callback_count,
                source_frames = request.audio.frame_count(),
                "audio capture delivered fewer than two callbacks"
            );
        }
        if capture_wall_duration_ms > 0
            && capture_duration_ms.saturating_mul(2) < capture_wall_duration_ms
        {
            warn!(
                shortcut_id = %request.shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms,
                capture_wall_duration_ms,
                startup_latency_ms = request.audio.startup_latency_ms,
                first_callback_latency_ms = request.audio.first_callback_latency_ms,
                audio_callback_count = request.audio.audio_callback_count,
                stale_callback_count = request.audio.stale_callback_count,
                stale_samples = request.audio.stale_samples,
                "captured audio duration is much shorter than shortcut hold time"
            );
        }
        info!(
            shortcut_id = %request.shortcut_id,
            shortcut_name = %request.shortcut_name,
            model_id = %request.model_id,
            input = %request.audio.input_label,
            capture_duration_ms,
            capture_wall_duration_ms,
            startup_latency_ms = request.audio.startup_latency_ms,
            first_callback_latency_ms = request.audio.first_callback_latency_ms,
            audio_callback_count = request.audio.audio_callback_count,
            source_sample_rate = request.audio.sample_rate,
            source_channels = request.audio.channels,
            source_frames = request.audio.frame_count(),
            dropped_samples = request.audio.dropped_samples,
            missed_audio_chunks = request.audio.missed_chunks,
            stale_callback_count = request.audio.stale_callback_count,
            stale_samples = request.audio.stale_samples,
            audio_rms = audio_stats.rms,
            audio_peak = audio_stats.peak,
            prepared_duration_ms,
            whisper_samples = prepared.samples.len(),
            "captured audio prepared for whisper"
        );
        if let Err(error) = maybe_write_debug_audio(&request, &prepared) {
            warn!(?error, "failed to write debug audio files");
        }
        debug!(
            shortcut_id = %request.shortcut_id,
            shortcut_name = %request.shortcut_name,
            model_id = %request.model_id,
            model_path = %request.model_path.display(),
            language = %request.language,
            compute_backend = %request.compute_backend.as_str(),
            output = request.output.label(),
            input = %request.audio.input_label,
            capture_duration_ms,
            capture_wall_duration_ms,
            startup_latency_ms = request.audio.startup_latency_ms,
            first_callback_latency_ms = request.audio.first_callback_latency_ms,
            audio_callback_count = request.audio.audio_callback_count,
            source_sample_rate = request.audio.sample_rate,
            source_channels = request.audio.channels,
            source_frames = request.audio.frame_count(),
            dropped_samples = request.audio.dropped_samples,
            missed_audio_chunks = request.audio.missed_chunks,
            stale_callback_count = request.audio.stale_callback_count,
            stale_samples = request.audio.stale_samples,
            prepared_duration_ms,
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
        let segment_count = state
            .full_n_segments()
            .context("failed to read whisper segment count")?;
        for index in 0..segment_count {
            segments.push(TranscriptionSegment {
                start_centiseconds: state
                    .full_get_segment_t0(index)
                    .with_context(|| format!("failed to read whisper segment {index} start"))?,
                end_centiseconds: state
                    .full_get_segment_t1(index)
                    .with_context(|| format!("failed to read whisper segment {index} end"))?,
                text: state
                    .full_get_segment_text_lossy(index)
                    .with_context(|| format!("failed to read whisper segment {index} text"))?,
            });
        }

        let text = segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let source_frames = request.audio.frame_count();
        let startup_latency_ms = request.audio.startup_latency_ms;
        let first_callback_latency_ms = request.audio.first_callback_latency_ms;
        let audio_callback_count = request.audio.audio_callback_count;
        let dropped_samples = request.audio.dropped_samples;
        let missed_audio_chunks = request.audio.missed_chunks;
        let stale_callback_count = request.audio.stale_callback_count;
        let stale_samples = request.audio.stale_samples;

        let result = TranscriptionResult {
            status: TranscriptionStatus::Completed,
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
                capture_wall_duration_ms,
                startup_latency_ms,
                first_callback_latency_ms,
                audio_callback_count,
                input_label: request.audio.input_label,
                source_sample_rate: prepared.source_sample_rate,
                source_channels: prepared.source_channels,
                source_frames,
                dropped_samples,
                missed_audio_chunks,
                stale_callback_count,
                stale_samples,
                audio_rms: audio_stats.rms,
                audio_peak: audio_stats.peak,
                whisper_sample_rate: prepared.sample_rate,
                whisper_samples: prepared.samples.len(),
                prepared_duration_ms,
                inference_duration_ms,
            },
        };

        debug!(
            shortcut_id = %request.shortcut_id,
            model_id = %result.debug.model_id,
            language = %result.debug.language,
            text = %result.text,
            "recognized text from whisper engine"
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
                capture_wall_duration_ms = result.debug.capture_wall_duration_ms,
                prepared_duration_ms = result.debug.prepared_duration_ms,
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

    fn handle_cache_after_transcription(&mut self, result: &Result<TranscriptionResult>) {
        match result {
            Ok(result) if matches!(result.status, TranscriptionStatus::Skipped { .. }) => {
                self.clear_cached_context("unusable audio capture skipped");
            }
            Err(_) => {
                self.clear_cached_context("whisper transcription failed");
            }
            Ok(_) => {}
        }
    }
}

impl Default for WhisperEngine {
    fn default() -> Self {
        Self::new(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use shared::{ComputeBackend, OutputAction};

    use crate::audio::CapturedAudio;
    use crate::transcription::compute::context_params;
    use crate::transcription::debug_audio::millis_u64;
    use crate::transcription::types::TranscriptionSkipReason;

    #[test]
    fn skips_too_short_capture_before_loading_model() {
        let mut engine = WhisperEngine::new(true);
        let prepared = PreparedAudio {
            samples: vec![0.05; 8_000],
            source_sample_rate: 16_000,
            source_channels: 1,
            sample_rate: 16_000,
        };
        let request = transcription_request_from_prepared(
            "short",
            PathBuf::from("/tmp/myapp-model-that-should-not-be-loaded.bin"),
            "/tmp/short.wav",
            prepared,
            1,
        );

        let result = engine
            .transcribe(request)
            .expect("short captures should be skipped before model loading");

        assert!(result.text.is_empty());
        assert!(result.segments.is_empty());
        assert!(matches!(
            result.status,
            TranscriptionStatus::Skipped {
                reason: TranscriptionSkipReason::CaptureTooShort
            }
        ));
        assert_eq!(result.debug.inference_duration_ms, 0);
        assert_eq!(result.debug.capture_duration_ms, 500);
    }

    #[test]
    #[ignore = "set MYAPP_TEST_WHISPER_MODEL and MYAPP_TEST_WHISPER_WAV to run manually"]
    fn debug_whisper_wav_transcribes_with_app_params() {
        let model_path =
            PathBuf::from(env::var_os("MYAPP_TEST_WHISPER_MODEL").expect(
                "MYAPP_TEST_WHISPER_MODEL must point to a downloaded whisper.cpp ggml model",
            ));
        let wav_path = PathBuf::from(
            env::var_os("MYAPP_TEST_WHISPER_WAV")
                .expect("MYAPP_TEST_WHISPER_WAV must point to a 16 kHz mono f32 debug WAV"),
        );
        let prepared = read_debug_whisper_wav(&wav_path);
        let now = Instant::now();
        let stopped_at = now + Duration::from_millis(millis_u64(prepared.duration_ms()));
        let request = TranscriptionRequest {
            recording_id: 1,
            shortcut_id: "debug".to_string(),
            shortcut_name: "Debug".to_string(),
            model_id: "debug-model".to_string(),
            model_path: model_path.clone(),
            language: "auto".to_string(),
            compute_backend: ComputeBackend::Auto,
            output: OutputAction::default(),
            audio: CapturedAudio {
                samples: prepared.samples.clone(),
                sample_rate: prepared.sample_rate,
                channels: 1,
                input_label: wav_path.display().to_string(),
                started_at: now,
                stopped_at,
                startup_latency_ms: 0,
                first_callback_latency_ms: Some(0),
                audio_callback_count: 1,
                dropped_samples: 0,
                missed_chunks: 0,
                stale_callback_count: 0,
                stale_samples: 0,
            },
        };
        let model_path = model_path
            .to_str()
            .expect("MYAPP_TEST_WHISPER_MODEL must be valid UTF-8");
        let context = WhisperContext::new_with_params(
            model_path,
            context_params(ComputeBackend::Auto).expect("debug context params should resolve"),
        )
        .expect("debug model should load");
        let result = WhisperEngine::run_transcription(&context, request, prepared)
            .expect("debug WAV should transcribe");

        assert!(
            !result.text.trim().is_empty(),
            "debug WAV produced no text; inspect audio and Whisper params"
        );
    }

    #[test]
    #[ignore = "set MYAPP_TEST_WHISPER_MODEL and MYAPP_TEST_WHISPER_WAV to run manually"]
    fn debug_whisper_cached_repeated_transcription_survives_short_skip() {
        let model_path =
            PathBuf::from(env::var_os("MYAPP_TEST_WHISPER_MODEL").expect(
                "MYAPP_TEST_WHISPER_MODEL must point to a downloaded whisper.cpp ggml model",
            ));
        let wav_path = PathBuf::from(
            env::var_os("MYAPP_TEST_WHISPER_WAV")
                .expect("MYAPP_TEST_WHISPER_WAV must point to a 16 kHz mono f32 debug WAV"),
        );
        let normal_prepared = read_debug_whisper_wav(&wav_path);
        let short_prepared = PreparedAudio {
            samples: vec![0.05; 8_000],
            source_sample_rate: 16_000,
            source_channels: 1,
            sample_rate: 16_000,
        };
        let mut engine = WhisperEngine::new(true);

        let skipped = engine
            .transcribe(transcription_request_from_prepared(
                "short",
                model_path.clone(),
                "/tmp/short.wav",
                short_prepared,
                1,
            ))
            .expect("short capture should be skipped");
        assert!(skipped.text.trim().is_empty());
        assert!(matches!(
            skipped.status,
            TranscriptionStatus::Skipped {
                reason: TranscriptionSkipReason::CaptureTooShort
            }
        ));
        assert_eq!(skipped.debug.inference_duration_ms, 0);

        let first = engine
            .transcribe(transcription_request_from_prepared(
                "debug",
                model_path.clone(),
                &wav_path.display().to_string(),
                normal_prepared.clone(),
                5,
            ))
            .expect("first normal transcription should succeed");
        assert!(
            !first.text.trim().is_empty(),
            "first normal transcription produced no text"
        );

        let second = engine
            .transcribe(transcription_request_from_prepared(
                "debug",
                model_path,
                &wav_path.display().to_string(),
                normal_prepared,
                5,
            ))
            .expect("cached normal transcription should succeed");
        assert!(
            !second.text.trim().is_empty(),
            "cached normal transcription produced no text"
        );
    }

    fn transcription_request_from_prepared(
        shortcut_id: &str,
        model_path: PathBuf,
        input_label: &str,
        prepared: PreparedAudio,
        audio_callback_count: u64,
    ) -> TranscriptionRequest {
        let now = Instant::now();
        let stopped_at = now + Duration::from_millis(millis_u64(prepared.duration_ms()));

        TranscriptionRequest {
            recording_id: 1,
            shortcut_id: shortcut_id.to_string(),
            shortcut_name: "Debug".to_string(),
            model_id: "debug-model".to_string(),
            model_path,
            language: "auto".to_string(),
            compute_backend: ComputeBackend::Auto,
            output: OutputAction::default(),
            audio: CapturedAudio {
                samples: prepared.samples,
                sample_rate: prepared.sample_rate,
                channels: 1,
                input_label: input_label.to_string(),
                started_at: now,
                stopped_at,
                startup_latency_ms: 0,
                first_callback_latency_ms: Some(0),
                audio_callback_count,
                dropped_samples: 0,
                missed_chunks: 0,
                stale_callback_count: 0,
                stale_samples: 0,
            },
        }
    }

    fn read_debug_whisper_wav(path: &Path) -> PreparedAudio {
        let mut reader = hound::WavReader::open(path).expect("debug WAV should be readable");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "debug WAV must be mono");
        assert_eq!(spec.sample_rate, 16_000, "debug WAV must be 16 kHz");
        assert_eq!(
            spec.sample_format,
            hound::SampleFormat::Float,
            "debug WAV must be f32"
        );
        let samples = reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .expect("debug WAV samples should be readable");

        PreparedAudio {
            samples,
            source_sample_rate: spec.sample_rate,
            source_channels: spec.channels,
            sample_rate: spec.sample_rate,
        }
    }
}
