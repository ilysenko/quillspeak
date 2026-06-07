use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream};
use shared::AudioInputRef;
use tracing::warn;

use crate::audio::devices::resolve_input_device;

const MAX_CAPTURE_SECONDS: usize = 10 * 60;

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub input_label: String,
    pub started_at: Instant,
    pub stopped_at: Instant,
    pub dropped_samples: u64,
    pub missed_chunks: u64,
}

impl CapturedAudio {
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        }
    }

    pub fn duration_ms(&self) -> u128 {
        self.stopped_at.duration_since(self.started_at).as_millis()
    }
}

pub struct AudioCaptureService {
    stream: Stream,
    buffer: Arc<Mutex<CaptureBuffer>>,
    missed_chunks: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
    input_label: String,
    started_at: Instant,
}

impl AudioCaptureService {
    pub fn start(input: &AudioInputRef) -> Result<Self> {
        let (device, input_label) = resolve_input_device(input)?;
        let config = device
            .default_input_config()
            .with_context(|| format!("failed to read default input config for {input_label}"))?;
        let sample_rate = config.sample_rate();
        let channels = config.channels();
        anyhow::ensure!(sample_rate > 0, "input device reported zero sample rate");
        anyhow::ensure!(channels > 0, "input device reported zero channels");
        let stream_config = config.into();
        let buffer = Arc::new(Mutex::new(CaptureBuffer::new(max_capture_samples(
            sample_rate,
            channels,
        ))));
        let missed_chunks = Arc::new(AtomicU64::new(0));
        let err_fn = {
            let input_label = input_label.clone();
            move |error| warn!(?error, input = input_label, "audio input stream error")
        };

        let stream = match config.sample_format() {
            SampleFormat::I8 => build_stream::<i8>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::I32 => build_stream::<i32>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::U8 => build_stream::<u8>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::U32 => build_stream::<u32>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            SampleFormat::F64 => build_stream::<f64>(
                &device,
                stream_config,
                Arc::clone(&buffer),
                Arc::clone(&missed_chunks),
                err_fn,
            ),
            sample_format => anyhow::bail!("unsupported input sample format {sample_format}"),
        }?;

        let started_at = Instant::now();
        stream
            .play()
            .with_context(|| format!("failed to start input stream for {input_label}"))?;

        Ok(Self {
            stream,
            buffer,
            missed_chunks,
            sample_rate,
            channels,
            input_label,
            started_at,
        })
    }

    pub fn stop(self) -> CapturedAudio {
        let Self {
            stream,
            buffer,
            missed_chunks,
            sample_rate,
            channels,
            input_label,
            started_at,
        } = self;
        drop(stream);
        let stopped_at = Instant::now();
        let buffer = buffer
            .lock()
            .map(|buffer| buffer.snapshot())
            .unwrap_or_default();
        CapturedAudio {
            samples: buffer.samples,
            sample_rate,
            channels,
            input_label,
            started_at,
            stopped_at,
            dropped_samples: buffer.dropped_samples,
            missed_chunks: missed_chunks.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct CaptureBufferSnapshot {
    samples: Vec<f32>,
    dropped_samples: u64,
}

#[derive(Debug)]
struct CaptureBuffer {
    samples: Vec<f32>,
    max_samples: usize,
    dropped_samples: u64,
}

impl CaptureBuffer {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::new(),
            max_samples,
            dropped_samples: 0,
        }
    }

    fn push_samples<I>(&mut self, incoming_len: usize, samples: I)
    where
        I: IntoIterator<Item = f32>,
    {
        let remaining = self.max_samples.saturating_sub(self.samples.len());
        let accepted = remaining.min(incoming_len);
        self.samples.extend(samples.into_iter().take(accepted));
        self.dropped_samples += (incoming_len - accepted) as u64;
    }

    fn snapshot(&self) -> CaptureBufferSnapshot {
        CaptureBufferSnapshot {
            samples: self.samples.clone(),
            dropped_samples: self.dropped_samples,
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    buffer: Arc<Mutex<CaptureBuffer>>,
    missed_chunks: Arc<AtomicU64>,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            if let Ok(mut buffer) = buffer.try_lock() {
                buffer.push_samples(data.len(), data.iter().copied().map(f32::from_sample));
            } else {
                missed_chunks.fetch_add(1, Ordering::Relaxed);
            }
        },
        err_fn,
        None,
    )?)
}

fn max_capture_samples(sample_rate: u32, channels: u16) -> usize {
    (sample_rate as usize)
        .saturating_mul(usize::from(channels))
        .saturating_mul(MAX_CAPTURE_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_buffer_caps_samples_and_counts_drops() {
        let mut buffer = CaptureBuffer::new(3);

        buffer.push_samples(2, [0.1, 0.2]);
        buffer.push_samples(3, [0.3, 0.4, 0.5]);

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.samples, vec![0.1, 0.2, 0.3]);
        assert_eq!(snapshot.dropped_samples, 2);
    }

    #[test]
    fn max_capture_samples_uses_sample_rate_channels_and_duration() {
        assert_eq!(max_capture_samples(16_000, 1), 16_000 * MAX_CAPTURE_SECONDS);
        assert_eq!(
            max_capture_samples(48_000, 2),
            48_000 * 2 * MAX_CAPTURE_SECONDS
        );
    }
}
