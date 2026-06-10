use crate::audio::{AudioSignalStats, PreparedAudio};
use crate::transcription::types::{
    TranscriptionDebugInfo, TranscriptionRequest, TranscriptionResult, TranscriptionSkipReason,
    TranscriptionStatus,
};

const MIN_TRANSCRIBE_CAPTURE_MS: u128 = 1_000;

pub(super) fn pad_short_whisper_audio(prepared: &mut PreparedAudio) {
    let minimum_samples = prepared.sample_rate as usize;
    if prepared.samples.len() >= minimum_samples {
        return;
    }

    prepared.samples.resize(minimum_samples, 0.0);
}

pub(super) fn skip_transcription_reason(
    request: &TranscriptionRequest,
    prepared: &PreparedAudio,
) -> Option<TranscriptionSkipReason> {
    let audio_stats = request.audio.signal_stats();
    if request.audio.duration_ms() < MIN_TRANSCRIBE_CAPTURE_MS {
        return Some(TranscriptionSkipReason::CaptureTooShort);
    }

    if prepared.duration_ms() < MIN_TRANSCRIBE_CAPTURE_MS {
        return Some(TranscriptionSkipReason::PreparedAudioTooShort);
    }

    if audio_stats.is_near_silent() {
        return Some(TranscriptionSkipReason::NearSilent);
    }

    None
}

pub(super) fn skipped_transcription_result(
    request: TranscriptionRequest,
    prepared: PreparedAudio,
    audio_stats: AudioSignalStats,
    reason: TranscriptionSkipReason,
) -> TranscriptionResult {
    let capture_duration_ms = request.audio.duration_ms();
    let capture_wall_duration_ms = request.audio.wall_duration_ms();
    let source_frames = request.audio.frame_count();
    let startup_latency_ms = request.audio.startup_latency_ms;
    let first_callback_latency_ms = request.audio.first_callback_latency_ms;
    let audio_callback_count = request.audio.audio_callback_count;
    let dropped_samples = request.audio.dropped_samples;
    let missed_audio_chunks = request.audio.missed_chunks;
    let stale_callback_count = request.audio.stale_callback_count;
    let stale_samples = request.audio.stale_samples;
    let output_label = request.output.label().to_string();

    TranscriptionResult {
        status: TranscriptionStatus::Skipped { reason },
        text: String::new(),
        segments: Vec::new(),
        output: request.output,
        debug: TranscriptionDebugInfo {
            shortcut_name: request.shortcut_name,
            model_id: request.model_id,
            model_path: request.model_path,
            language: request.language,
            compute_backend: request.compute_backend,
            output_label,
            input_label: request.audio.input_label,
            capture_duration_ms,
            capture_wall_duration_ms,
            startup_latency_ms,
            first_callback_latency_ms,
            audio_callback_count,
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
            prepared_duration_ms: prepared.duration_ms(),
            inference_duration_ms: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use shared::{ComputeBackend, OutputAction};

    use crate::audio::CapturedAudio;

    #[test]
    fn pads_short_whisper_audio_to_one_second() {
        let mut prepared = PreparedAudio {
            samples: vec![0.25; 5_461],
            source_sample_rate: 48_000,
            source_channels: 2,
            sample_rate: 16_000,
        };

        pad_short_whisper_audio(&mut prepared);

        assert_eq!(prepared.samples.len(), 16_000);
        assert_eq!(prepared.samples[0], 0.25);
        assert_eq!(prepared.samples[5_460], 0.25);
        assert_eq!(prepared.samples[5_461], 0.0);
    }

    #[test]
    fn leaves_long_enough_whisper_audio_unchanged() {
        let mut prepared = PreparedAudio {
            samples: vec![0.25; 16_000],
            source_sample_rate: 48_000,
            source_channels: 2,
            sample_rate: 16_000,
        };

        pad_short_whisper_audio(&mut prepared);

        assert_eq!(prepared.samples.len(), 16_000);
        assert!(prepared.samples.iter().all(|sample| *sample == 0.25));
    }

    #[test]
    fn skips_near_silent_capture_after_minimum_duration() {
        let prepared = prepared_audio_with_samples(vec![0.0; 16_000]);
        let request = request_from_prepared(&prepared);

        assert_eq!(
            skip_transcription_reason(&request, &prepared),
            Some(TranscriptionSkipReason::NearSilent)
        );
    }

    #[test]
    fn audible_capture_is_not_skipped_by_near_silent_threshold() {
        let prepared = prepared_audio_with_samples(vec![0.02; 16_000]);
        let request = request_from_prepared(&prepared);

        assert_eq!(skip_transcription_reason(&request, &prepared), None);
    }

    fn prepared_audio_with_samples(samples: Vec<f32>) -> PreparedAudio {
        PreparedAudio {
            samples,
            source_sample_rate: 16_000,
            source_channels: 1,
            sample_rate: 16_000,
        }
    }

    fn request_from_prepared(prepared: &PreparedAudio) -> TranscriptionRequest {
        let now = Instant::now();
        let stopped_at = now + Duration::from_millis(prepared.duration_ms() as u64);
        TranscriptionRequest {
            recording_id: 1,
            shortcut_id: "default".to_string(),
            shortcut_name: "Default".to_string(),
            model_id: "debug-model".to_string(),
            model_path: PathBuf::from("/tmp/debug-model.bin"),
            language: "auto".to_string(),
            compute_backend: ComputeBackend::Auto,
            output: OutputAction::default(),
            audio: CapturedAudio {
                samples: prepared.samples.clone(),
                sample_rate: prepared.sample_rate,
                channels: prepared.source_channels,
                input_label: "Debug input".to_string(),
                started_at: now,
                stopped_at,
                startup_latency_ms: 0,
                first_callback_latency_ms: Some(0),
                audio_callback_count: 2,
                dropped_samples: 0,
                missed_chunks: 0,
                stale_callback_count: 0,
                stale_samples: 0,
            },
        }
    }
}
