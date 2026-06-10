use std::path::PathBuf;
use std::time::{Duration, Instant};

use shared::{ComputeBackend, OutputAction};

use crate::audio::{CapturedAudio, PreparedAudio};
use crate::transcription::debug_audio::millis_u64;
use crate::transcription::skip::skipped_transcription_result;
use crate::transcription::types::{
    TranscriptionRequest, TranscriptionResult, TranscriptionSkipReason,
};

pub(crate) fn prepared_audio_with_samples(samples: Vec<f32>) -> PreparedAudio {
    PreparedAudio {
        samples,
        source_sample_rate: 16_000,
        source_channels: 1,
        sample_rate: 16_000,
    }
}

pub(crate) fn transcription_request_from_prepared(
    shortcut_id: &str,
    model_path: PathBuf,
    input_label: &str,
    prepared: &PreparedAudio,
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
        beep_on_recording: false,
        output: OutputAction::default(),
        audio: CapturedAudio {
            samples: prepared.samples.clone(),
            sample_rate: prepared.sample_rate,
            channels: prepared.source_channels,
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

pub(crate) fn skipped_transcription_result_fixture(
    reason: TranscriptionSkipReason,
) -> TranscriptionResult {
    let prepared = prepared_audio_with_samples(vec![0.0; 16_000]);
    let request = transcription_request_from_prepared(
        "default",
        PathBuf::from("/tmp/debug-model.bin"),
        "Debug input",
        &prepared,
        2,
    );
    let audio_stats = request.audio.signal_stats();
    skipped_transcription_result(request, prepared, audio_stats, reason)
}
