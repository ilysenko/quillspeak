use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream};
use rtrb::{Consumer, Producer, RingBuffer};
use shared::AudioInputRef;
use tracing::{debug, warn};

use crate::audio::devices::resolve_input_device;

const MAX_CAPTURE_SECONDS: usize = 10 * 60;
const FIRST_CALLBACK_WAIT_TIMEOUT: Duration = Duration::from_millis(500);
const FIRST_CALLBACK_WAIT_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub input_label: String,
    pub started_at: Instant,
    pub stopped_at: Instant,
    pub startup_latency_ms: u128,
    pub first_callback_latency_ms: Option<u128>,
    pub audio_callback_count: u64,
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
        let frames = self.frame_count() as u128;
        if self.sample_rate == 0 {
            0
        } else {
            frames.saturating_mul(1000) / u128::from(self.sample_rate)
        }
    }

    pub fn wall_duration_ms(&self) -> u128 {
        self.stopped_at.duration_since(self.started_at).as_millis()
    }

    pub fn signal_stats(&self) -> AudioSignalStats {
        AudioSignalStats::from_samples(&self.samples)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioSignalStats {
    pub rms: f32,
    pub peak: f32,
}

impl AudioSignalStats {
    fn from_samples(samples: &[f32]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }

        let mut sum_squares = 0.0f64;
        let mut peak = 0.0f32;
        for sample in samples {
            let absolute = sample.abs();
            peak = peak.max(absolute);
            sum_squares += f64::from(*sample) * f64::from(*sample);
        }

        Self {
            rms: (sum_squares / samples.len() as f64).sqrt() as f32,
            peak,
        }
    }

    pub fn is_near_silent(self) -> bool {
        self.rms < 0.001 && self.peak < 0.01
    }
}

pub struct AudioCaptureService {
    stream: Stream,
    input: AudioInputRef,
    consumer: Consumer<f32>,
    active: Arc<AtomicBool>,
    callback_count: Arc<AtomicU64>,
    dropped_samples: Arc<AtomicU64>,
    missed_chunks: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
    input_label: String,
    session_started_at: Option<Instant>,
    startup_latency_ms: u128,
    first_callback_latency_ms: Option<u128>,
    stream_running: bool,
}

impl AudioCaptureService {
    pub fn for_input(input: &AudioInputRef) -> Result<Self> {
        let (device, input_label) = resolve_input_device(input)?;
        let config = device
            .default_input_config()
            .with_context(|| format!("failed to read default input config for {input_label}"))?;
        let sample_rate = config.sample_rate();
        let channels = config.channels();
        let sample_format = config.sample_format();
        anyhow::ensure!(sample_rate > 0, "input device reported zero sample rate");
        anyhow::ensure!(channels > 0, "input device reported zero channels");
        let stream_config = config.into();
        let capacity = max_capture_samples(sample_rate, channels);
        let (producer, consumer) = RingBuffer::new(capacity);
        let active = Arc::new(AtomicBool::new(false));
        let callback_count = Arc::new(AtomicU64::new(0));
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let missed_chunks = Arc::new(AtomicU64::new(0));
        let callback_state = AudioCallbackState::new(
            producer,
            Arc::clone(&active),
            Arc::clone(&callback_count),
            Arc::clone(&dropped_samples),
            Arc::clone(&missed_chunks),
        );
        let err_fn = {
            let input_label = input_label.clone();
            move |error| warn!(?error, input = input_label, "audio input stream error")
        };

        let stream = match sample_format {
            SampleFormat::I8 => build_stream::<i8>(&device, stream_config, callback_state, err_fn),
            SampleFormat::I16 => {
                build_stream::<i16>(&device, stream_config, callback_state, err_fn)
            }
            SampleFormat::I32 => {
                build_stream::<i32>(&device, stream_config, callback_state, err_fn)
            }
            SampleFormat::U8 => build_stream::<u8>(&device, stream_config, callback_state, err_fn),
            SampleFormat::U16 => {
                build_stream::<u16>(&device, stream_config, callback_state, err_fn)
            }
            SampleFormat::U32 => {
                build_stream::<u32>(&device, stream_config, callback_state, err_fn)
            }
            SampleFormat::F32 => {
                build_stream::<f32>(&device, stream_config, callback_state, err_fn)
            }
            SampleFormat::F64 => {
                build_stream::<f64>(&device, stream_config, callback_state, err_fn)
            }
            sample_format => anyhow::bail!("unsupported input sample format {sample_format}"),
        }?;

        Ok(Self {
            stream,
            input: input.clone(),
            consumer,
            active,
            callback_count,
            dropped_samples,
            missed_chunks,
            sample_rate,
            channels,
            input_label,
            session_started_at: None,
            startup_latency_ms: 0,
            first_callback_latency_ms: None,
            stream_running: false,
        })
    }

    pub fn input(&self) -> &AudioInputRef {
        &self.input
    }

    pub fn input_label(&self) -> &str {
        &self.input_label
    }

    pub fn start_session(&mut self) -> Result<AudioCaptureStartInfo> {
        self.drain_samples();
        self.dropped_samples.store(0, Ordering::Relaxed);
        self.missed_chunks.store(0, Ordering::Relaxed);
        self.callback_count.store(0, Ordering::Relaxed);

        let startup_started_at = Instant::now();
        self.session_started_at = Some(startup_started_at);
        self.first_callback_latency_ms = None;
        self.active.store(true, Ordering::Release);
        if let Err(error) = self.ensure_stream_running() {
            self.active.store(false, Ordering::Release);
            self.session_started_at = None;
            return Err(error);
        }
        let first_callback_latency_ms = self.wait_for_first_callback(startup_started_at);
        let startup_latency_ms = startup_started_at.elapsed().as_millis();
        self.startup_latency_ms = startup_latency_ms;
        self.first_callback_latency_ms = first_callback_latency_ms;

        Ok(AudioCaptureStartInfo {
            input_label: self.input_label.clone(),
            startup_latency_ms,
            first_callback_latency_ms,
        })
    }

    pub fn stop_session(&mut self) -> AudioCaptureStopInfo {
        self.active.store(false, Ordering::Release);
        let pause_error = self.pause_stream().err().map(|error| format!("{error:#}"));

        let stopped_at = Instant::now();
        let samples = self.drain_samples();
        let started_at = self.session_started_at.take().unwrap_or(stopped_at);
        let audio = CapturedAudio {
            samples,
            sample_rate: self.sample_rate,
            channels: self.channels,
            input_label: self.input_label.clone(),
            started_at,
            stopped_at,
            startup_latency_ms: self.startup_latency_ms,
            first_callback_latency_ms: self.first_callback_latency_ms,
            audio_callback_count: self.callback_count.load(Ordering::Relaxed),
            dropped_samples: self.dropped_samples.load(Ordering::Relaxed),
            missed_chunks: self.missed_chunks.load(Ordering::Relaxed),
        };

        AudioCaptureStopInfo { audio, pause_error }
    }

    fn ensure_stream_running(&mut self) -> Result<()> {
        if self.stream_running {
            return Ok(());
        }

        self.stream
            .play()
            .with_context(|| format!("failed to start input stream for {}", self.input_label))?;
        self.stream_running = true;
        Ok(())
    }

    fn pause_stream(&mut self) -> Result<()> {
        if !self.stream_running {
            return Ok(());
        }

        self.stream
            .pause()
            .with_context(|| format!("failed to pause input stream for {}", self.input_label))?;
        self.stream_running = false;
        Ok(())
    }

    fn wait_for_first_callback(&self, started_at: Instant) -> Option<u128> {
        while started_at.elapsed() < FIRST_CALLBACK_WAIT_TIMEOUT {
            if self.callback_count.load(Ordering::Acquire) > 0 {
                return Some(started_at.elapsed().as_millis());
            }
            thread::sleep(FIRST_CALLBACK_WAIT_INTERVAL);
        }

        debug!(
            input = %self.input_label,
            timeout_ms = FIRST_CALLBACK_WAIT_TIMEOUT.as_millis(),
            "audio input stream did not deliver a callback before startup wait timeout"
        );
        None
    }

    fn drain_samples(&mut self) -> Vec<f32> {
        let available = self.consumer.slots();
        let mut samples = Vec::with_capacity(available);
        while let Ok(sample) = self.consumer.pop() {
            samples.push(sample);
        }
        samples
    }
}

#[derive(Debug)]
pub struct AudioCaptureStartInfo {
    pub input_label: String,
    pub startup_latency_ms: u128,
    pub first_callback_latency_ms: Option<u128>,
}

#[derive(Debug)]
pub struct AudioCaptureStopInfo {
    pub audio: CapturedAudio,
    pub pause_error: Option<String>,
}

struct AudioCallbackState {
    producer: Producer<f32>,
    active: Arc<AtomicBool>,
    callback_count: Arc<AtomicU64>,
    dropped_samples: Arc<AtomicU64>,
    missed_chunks: Arc<AtomicU64>,
}

impl AudioCallbackState {
    fn new(
        producer: Producer<f32>,
        active: Arc<AtomicBool>,
        callback_count: Arc<AtomicU64>,
        dropped_samples: Arc<AtomicU64>,
        missed_chunks: Arc<AtomicU64>,
    ) -> Self {
        Self {
            producer,
            active,
            callback_count,
            dropped_samples,
            missed_chunks,
        }
    }

    fn push_samples<T>(&mut self, samples: &[T])
    where
        T: Sample + Copy,
        f32: FromSample<T>,
    {
        if !self.active.load(Ordering::Acquire) {
            return;
        }

        self.callback_count.fetch_add(1, Ordering::Relaxed);
        let mut dropped_in_callback = 0u64;
        for sample in samples {
            if self.producer.push(f32::from_sample(*sample)).is_err() {
                dropped_in_callback += 1;
            }
        }

        if dropped_in_callback > 0 {
            self.dropped_samples
                .fetch_add(dropped_in_callback, Ordering::Relaxed);
            self.missed_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut callback_state: AudioCallbackState,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            callback_state.push_samples(data);
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
    fn audio_callback_state_caps_samples_and_counts_drops() {
        let (producer, mut consumer) = RingBuffer::new(3);
        let active = Arc::new(AtomicBool::new(true));
        let callback_count = Arc::new(AtomicU64::new(0));
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let missed_chunks = Arc::new(AtomicU64::new(0));
        let mut state = AudioCallbackState::new(
            producer,
            Arc::clone(&active),
            Arc::clone(&callback_count),
            Arc::clone(&dropped_samples),
            Arc::clone(&missed_chunks),
        );

        state.push_samples(&[0.1f32, 0.2]);
        state.push_samples(&[0.3f32, 0.4, 0.5]);

        let mut samples = Vec::new();
        while let Ok(sample) = consumer.pop() {
            samples.push(sample);
        }
        assert_eq!(samples, vec![0.1, 0.2, 0.3]);
        assert_eq!(dropped_samples.load(Ordering::Relaxed), 2);
        assert_eq!(missed_chunks.load(Ordering::Relaxed), 1);
        assert_eq!(callback_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn max_capture_samples_uses_sample_rate_channels_and_duration() {
        assert_eq!(max_capture_samples(16_000, 1), 16_000 * MAX_CAPTURE_SECONDS);
        assert_eq!(
            max_capture_samples(48_000, 2),
            48_000 * 2 * MAX_CAPTURE_SECONDS
        );
    }

    #[test]
    fn audio_signal_stats_detect_peak_and_rms() {
        let stats = AudioSignalStats::from_samples(&[0.0, 0.5, -0.5, 0.0]);

        assert_eq!(stats.peak, 0.5);
        assert!((stats.rms - 0.35355338).abs() < 0.000001);
        assert!(!stats.is_near_silent());
        assert!(AudioSignalStats::from_samples(&[0.0; 16]).is_near_silent());
    }
}
