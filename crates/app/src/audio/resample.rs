use crate::audio::CapturedAudio;

use anyhow::{Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

pub const WHISPER_SAMPLE_RATE: u32 = 16_000;
const RESAMPLE_CHUNK_SIZE: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedAudio {
    pub samples: Vec<f32>,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub sample_rate: u32,
}

impl PreparedAudio {
    pub fn duration_ms(&self) -> u128 {
        if self.sample_rate == 0 {
            0
        } else {
            (self.samples.len() as u128).saturating_mul(1000) / u128::from(self.sample_rate)
        }
    }
}

pub fn prepare_whisper_audio(audio: &CapturedAudio) -> Result<PreparedAudio> {
    let mono = mix_to_mono(&audio.samples, audio.channels);
    let samples = resample_to_rate(&mono, audio.sample_rate, WHISPER_SAMPLE_RATE)?;
    Ok(PreparedAudio {
        samples,
        source_sample_rate: audio.sample_rate,
        source_channels: audio.channels,
        sample_rate: WHISPER_SAMPLE_RATE,
    })
}

fn mix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    match channels {
        0 => Vec::new(),
        1 => samples.to_vec(),
        channels => {
            let channels = usize::from(channels);
            samples
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect()
        }
    }
}

fn resample_to_rate(samples: &[f32], source_rate: u32, target_rate: u32) -> Result<Vec<f32>> {
    if samples.is_empty() || source_rate == 0 || target_rate == 0 {
        return Ok(Vec::new());
    }
    if source_rate == target_rate {
        return Ok(samples.to_vec());
    }

    let input_frames = samples.len();
    let input = InterleavedSlice::new(samples, 1, input_frames)
        .context("failed to create rubato input adapter")?;
    let mut resampler = Fft::<f32>::new(
        source_rate as usize,
        target_rate as usize,
        RESAMPLE_CHUNK_SIZE,
        2,
        1,
        FixedSync::Both,
    )
    .context("failed to create rubato resampler")?;
    let output_len = resampler.process_all_needed_output_len(input_frames);
    let mut output = vec![0.0; output_len];
    let mut output_adapter = InterleavedSlice::new_mut(&mut output, 1, output_len)
        .context("failed to create rubato output adapter")?;
    let (_, output_frames) = resampler
        .process_all_into_buffer(&input, &mut output_adapter, input_frames, None)
        .context("failed to resample audio for whisper")?;

    output.truncate(output_frames);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn mixes_stereo_to_mono() {
        assert_eq!(mix_to_mono(&[1.0, 0.0, 0.5, -0.5], 2), vec![0.5, 0.0]);
    }

    #[test]
    fn preserves_audio_when_sample_rate_matches() {
        let input = vec![0.0, 0.25, -0.25];

        assert_eq!(resample_to_rate(&input, 16_000, 16_000).unwrap(), input);
    }

    #[test]
    fn prepares_whisper_audio_as_16khz_mono() {
        let now = Instant::now();
        let audio = CapturedAudio {
            samples: vec![0.0; 48_000 * 2],
            sample_rate: 48_000,
            channels: 2,
            input_label: "test".to_string(),
            started_at: now,
            stopped_at: now,
            startup_latency_ms: 0,
            first_callback_latency_ms: Some(0),
            audio_callback_count: 1,
            dropped_samples: 0,
            missed_chunks: 0,
            stale_callback_count: 0,
            stale_samples: 0,
        };

        let prepared = prepare_whisper_audio(&audio).unwrap();

        assert_eq!(prepared.sample_rate, WHISPER_SAMPLE_RATE);
        assert_eq!(prepared.source_sample_rate, 48_000);
        assert_eq!(prepared.source_channels, 2);
        assert_eq!(prepared.samples.len(), 16_000);
        assert_eq!(prepared.duration_ms(), 1_000);
    }

    #[test]
    fn resamples_44100hz_mono_to_16khz_duration() {
        let input = vec![0.1; 44_100];

        let output = resample_to_rate(&input, 44_100, WHISPER_SAMPLE_RATE).unwrap();

        assert!(output.len().abs_diff(16_000) <= 1);
    }
}
