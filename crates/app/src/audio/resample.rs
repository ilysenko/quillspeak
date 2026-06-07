use crate::audio::CapturedAudio;

pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

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

pub fn prepare_whisper_audio(audio: &CapturedAudio) -> PreparedAudio {
    let mono = mix_to_mono(&audio.samples, audio.channels);
    let samples = resample_linear(&mono, audio.sample_rate, WHISPER_SAMPLE_RATE);
    PreparedAudio {
        samples,
        source_sample_rate: audio.sample_rate,
        source_channels: audio.channels,
        sample_rate: WHISPER_SAMPLE_RATE,
    }
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

fn resample_linear(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate == 0 || target_rate == 0 {
        return Vec::new();
    }
    if source_rate == target_rate {
        return samples.to_vec();
    }

    let ratio = source_rate as f64 / target_rate as f64;
    let output_len = ((samples.len() as f64) / ratio).round().max(1.0) as usize;
    let last_index = samples.len() - 1;
    let mut output = Vec::with_capacity(output_len);

    for index in 0..output_len {
        let source_position = index as f64 * ratio;
        let left = source_position.floor() as usize;
        let right = (left + 1).min(last_index);
        let fraction = (source_position - left as f64) as f32;
        let sample = samples[left] * (1.0 - fraction) + samples[right] * fraction;
        output.push(sample);
    }

    output
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

        assert_eq!(resample_linear(&input, 16_000, 16_000), input);
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
        };

        let prepared = prepare_whisper_audio(&audio);

        assert_eq!(prepared.sample_rate, WHISPER_SAMPLE_RATE);
        assert_eq!(prepared.source_sample_rate, 48_000);
        assert_eq!(prepared.source_channels, 2);
        assert_eq!(prepared.samples.len(), 16_000);
        assert_eq!(prepared.duration_ms(), 1_000);
    }
}
