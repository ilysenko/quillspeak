use crate::audio::{AudioSignalStats, PreparedAudio};
use crate::transcription::types::{
    TranscriptionDebugInfo, TranscriptionRequest, TranscriptionResult, TranscriptionSkipReason,
    TranscriptionStatus,
};

const MIN_TRANSCRIBE_CAPTURE_MS: u128 = 1_000;
const MIN_AUDIO_CALLBACK_COUNT: u64 = 2;

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
    if request.audio.duration_ms() < MIN_TRANSCRIBE_CAPTURE_MS {
        return Some(TranscriptionSkipReason::CaptureTooShort);
    }

    if request.audio.audio_callback_count < MIN_AUDIO_CALLBACK_COUNT {
        return Some(TranscriptionSkipReason::TooFewAudioCallbacks);
    }

    if prepared.duration_ms() < MIN_TRANSCRIBE_CAPTURE_MS {
        return Some(TranscriptionSkipReason::PreparedAudioTooShort);
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
}
