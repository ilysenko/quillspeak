use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::info;

use crate::audio::PreparedAudio;
use crate::transcription::types::TranscriptionRequest;

pub(super) fn maybe_write_debug_audio(
    request: &TranscriptionRequest,
    prepared: &PreparedAudio,
) -> Result<()> {
    let Some(dir) = debug_audio_dir() else {
        return Ok(());
    };

    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create debug audio directory {}", dir.display()))?;
    let base_name = format!(
        "{}-{}",
        sanitize_file_component(&request.shortcut_id),
        unix_timestamp_millis()
    );
    let source_path = dir.join(format!("{base_name}-source.wav"));
    let whisper_path = dir.join(format!("{base_name}-whisper-16k-mono.wav"));
    let metadata_path = dir.join(format!("{base_name}.toml"));

    write_f32_wav(
        &source_path,
        &request.audio.samples,
        request.audio.sample_rate,
        request.audio.channels,
    )
    .with_context(|| format!("failed to write source debug WAV {}", source_path.display()))?;
    write_f32_wav(&whisper_path, &prepared.samples, prepared.sample_rate, 1).with_context(
        || {
            format!(
                "failed to write whisper debug WAV {}",
                whisper_path.display()
            )
        },
    )?;

    let metadata = DebugAudioMetadata {
        shortcut_id: &request.shortcut_id,
        shortcut_name: &request.shortcut_name,
        model_id: &request.model_id,
        model_path: request.model_path.to_string_lossy().into_owned(),
        language: &request.language,
        compute_backend: request.compute_backend.as_str(),
        input_label: &request.audio.input_label,
        source_sample_rate: request.audio.sample_rate,
        source_channels: request.audio.channels,
        source_frames: request.audio.frame_count(),
        source_duration_ms: millis_u64(request.audio.duration_ms()),
        source_wall_duration_ms: millis_u64(request.audio.wall_duration_ms()),
        startup_latency_ms: millis_u64(request.audio.startup_latency_ms),
        first_callback_latency_ms: request.audio.first_callback_latency_ms.map(millis_u64),
        audio_callback_count: request.audio.audio_callback_count,
        dropped_samples: request.audio.dropped_samples,
        missed_audio_chunks: request.audio.missed_chunks,
        stale_callback_count: request.audio.stale_callback_count,
        stale_samples: request.audio.stale_samples,
        whisper_sample_rate: prepared.sample_rate,
        whisper_samples: prepared.samples.len(),
        prepared_duration_ms: millis_u64(prepared.duration_ms()),
        source_path: source_path.to_string_lossy().into_owned(),
        whisper_path: whisper_path.to_string_lossy().into_owned(),
    };
    let metadata_text = toml::to_string_pretty(&metadata)?;
    fs::write(&metadata_path, metadata_text)
        .with_context(|| format!("failed to write debug metadata {}", metadata_path.display()))?;

    info!(
        shortcut_id = %request.shortcut_id,
        model_id = %request.model_id,
        source_path = %source_path.display(),
        whisper_path = %whisper_path.display(),
        metadata_path = %metadata_path.display(),
        source_duration_ms = request.audio.duration_ms(),
        source_wall_duration_ms = request.audio.wall_duration_ms(),
        prepared_duration_ms = prepared.duration_ms(),
        "wrote debug audio files"
    );
    Ok(())
}

fn debug_audio_dir() -> Option<PathBuf> {
    let value = env::var_os("QUILLSPEAK_DEBUG_SAVE_AUDIO")?;
    if value.is_empty() {
        return None;
    }

    let value_text = value.to_string_lossy();
    match value_text.to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => None,
        "1" | "true" | "on" | "yes" => Some(env::temp_dir().join("quillspeak-audio-debug")),
        _ => Some(PathBuf::from(value)),
    }
}

#[derive(Serialize)]
struct DebugAudioMetadata<'a> {
    shortcut_id: &'a str,
    shortcut_name: &'a str,
    model_id: &'a str,
    model_path: String,
    language: &'a str,
    compute_backend: &'a str,
    input_label: &'a str,
    source_sample_rate: u32,
    source_channels: u16,
    source_frames: usize,
    source_duration_ms: u64,
    source_wall_duration_ms: u64,
    startup_latency_ms: u64,
    first_callback_latency_ms: Option<u64>,
    audio_callback_count: u64,
    dropped_samples: u64,
    missed_audio_chunks: u64,
    stale_callback_count: u64,
    stale_samples: u64,
    whisper_sample_rate: u32,
    whisper_samples: usize,
    prepared_duration_ms: u64,
    source_path: String,
    whisper_path: String,
}

pub(super) fn write_f32_wav(
    path: &Path,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<()> {
    anyhow::ensure!(sample_rate > 0, "debug WAV sample rate cannot be zero");
    anyhow::ensure!(channels > 0, "debug WAV channel count cannot be zero");

    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample(sample.clamp(-1.0, 1.0))?;
    }
    writer.finalize()?;
    Ok(())
}

fn sanitize_file_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "shortcut".to_string()
    } else {
        sanitized
    }
}

pub(super) fn millis_u64(value: u128) -> u64 {
    value.min(u128::from(u64::MAX)) as u64
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_file_component_keeps_debug_wav_names_safe() {
        assert_eq!(sanitize_file_component("default"), "default");
        assert_eq!(sanitize_file_component("Ctrl+Alt Space"), "Ctrl_Alt_Space");
        assert_eq!(sanitize_file_component(""), "shortcut");
    }

    #[test]
    fn write_f32_wav_writes_readable_float_wav() {
        let path = env::temp_dir().join(format!(
            "quillspeak-debug-wav-test-{}-{}.wav",
            std::process::id(),
            unix_timestamp_millis()
        ));

        write_f32_wav(&path, &[0.0, 1.0, -1.0], 16_000, 1).expect("debug WAV should be written");
        let mut reader = hound::WavReader::open(&path).expect("debug WAV should be readable");
        let spec = reader.spec();
        let samples = reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .expect("debug WAV samples should be readable");
        let _ = fs::remove_file(&path);

        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 32);
        assert_eq!(spec.sample_format, hound::SampleFormat::Float);
        assert_eq!(samples, vec![0.0, 1.0, -1.0]);
    }
}
