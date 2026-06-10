use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{
    BufferSize, FrameCount, FromSample, Sample, SampleFormat, Stream, StreamInstant,
    SupportedBufferSize,
};
use rtrb::{Consumer, Producer, RingBuffer};
use shared::AudioInputRef;
use tracing::warn;

use crate::audio::devices::resolve_input_device;

const MAX_CAPTURE_SECONDS: usize = 60;
const CALLBACK_BUFFER_SECONDS: usize = 2;
const SESSION_PREROLL_ALLOWANCE: Duration = Duration::from_millis(50);
const TARGET_INPUT_BUFFER_FRAMES: FrameCount = 1024;
const NO_FIRST_CALLBACK_LATENCY: u64 = u64::MAX;

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
    pub stale_callback_count: u64,
    pub stale_samples: u64,
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
    consumer: Consumer<f32>,
    active: Arc<AtomicBool>,
    callback_count: Arc<AtomicU64>,
    dropped_samples: Arc<AtomicU64>,
    missed_chunks: Arc<AtomicU64>,
    stale_callback_count: Arc<AtomicU64>,
    stale_samples: Arc<AtomicU64>,
    session_capture_floor_nanos: Arc<AtomicU64>,
    session_callback_start_nanos: Arc<AtomicU64>,
    first_callback_latency_ms_atomic: Arc<AtomicU64>,
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
        let mut last_error = None;
        for stream_config in stream_config_attempts(&config) {
            let capacity = callback_buffer_samples(sample_rate, channels);
            let (producer, consumer) = RingBuffer::new(capacity);
            let active = Arc::new(AtomicBool::new(false));
            let callback_count = Arc::new(AtomicU64::new(0));
            let dropped_samples = Arc::new(AtomicU64::new(0));
            let missed_chunks = Arc::new(AtomicU64::new(0));
            let stale_callback_count = Arc::new(AtomicU64::new(0));
            let stale_samples = Arc::new(AtomicU64::new(0));
            let session_capture_floor_nanos = Arc::new(AtomicU64::new(0));
            let session_callback_start_nanos = Arc::new(AtomicU64::new(0));
            let first_callback_latency_ms_atomic =
                Arc::new(AtomicU64::new(NO_FIRST_CALLBACK_LATENCY));
            let callback_state_atomics = AudioCallbackStateAtomics {
                active: Arc::clone(&active),
                callback_count: Arc::clone(&callback_count),
                dropped_samples: Arc::clone(&dropped_samples),
                missed_chunks: Arc::clone(&missed_chunks),
                stale_callback_count: Arc::clone(&stale_callback_count),
                stale_samples: Arc::clone(&stale_samples),
                session_capture_floor_nanos: Arc::clone(&session_capture_floor_nanos),
                session_callback_start_nanos: Arc::clone(&session_callback_start_nanos),
                first_callback_latency_ms: Arc::clone(&first_callback_latency_ms_atomic),
            };
            let callback_state = AudioCallbackState::new(producer, callback_state_atomics);
            let err_fn = {
                let input_label = input_label.clone();
                move |error| warn!(?error, input = input_label, "audio input stream error")
            };

            let stream = match build_stream_for_format(
                sample_format,
                &device,
                stream_config.config,
                callback_state,
                err_fn,
            ) {
                Ok(stream) => stream,
                Err(error) => {
                    if stream_config.is_fallback {
                        return Err(error);
                    }
                    warn!(
                        ?error,
                        input = %input_label,
                        buffer_size = ?stream_config.config.buffer_size,
                        "failed to build low-latency input stream; retrying with default buffer size"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            return Ok(Self {
                stream,
                consumer,
                active,
                callback_count,
                dropped_samples,
                missed_chunks,
                stale_callback_count,
                stale_samples,
                session_capture_floor_nanos,
                session_callback_start_nanos,
                first_callback_latency_ms_atomic,
                sample_rate,
                channels,
                input_label,
                session_started_at: None,
                startup_latency_ms: 0,
                first_callback_latency_ms: None,
                stream_running: false,
            });
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no input stream config was attempted")))
    }

    pub fn input_label(&self) -> &str {
        &self.input_label
    }

    pub fn start_session(&mut self) -> Result<AudioCaptureStartInfo> {
        self.drain_samples();
        self.dropped_samples.store(0, Ordering::Relaxed);
        self.missed_chunks.store(0, Ordering::Relaxed);
        self.stale_callback_count.store(0, Ordering::Relaxed);
        self.stale_samples.store(0, Ordering::Relaxed);
        self.callback_count.store(0, Ordering::Relaxed);
        self.first_callback_latency_ms_atomic
            .store(NO_FIRST_CALLBACK_LATENCY, Ordering::Relaxed);

        let startup_started_at = Instant::now();
        self.session_started_at = Some(startup_started_at);
        self.first_callback_latency_ms = None;
        let stream_start = self.stream.now();
        let capture_floor = stream_start
            .checked_sub(SESSION_PREROLL_ALLOWANCE)
            .unwrap_or(StreamInstant::ZERO);
        self.session_capture_floor_nanos
            .store(stream_instant_nanos_u64(capture_floor), Ordering::Release);
        self.session_callback_start_nanos
            .store(stream_instant_nanos_u64(stream_start), Ordering::Release);
        self.active.store(true, Ordering::Release);
        if let Err(error) = self.ensure_stream_running() {
            self.active.store(false, Ordering::Release);
            self.session_started_at = None;
            return Err(error);
        }
        let startup_latency_ms = startup_started_at.elapsed().as_millis();
        self.startup_latency_ms = startup_latency_ms;

        Ok(AudioCaptureStartInfo {
            input_label: self.input_label.clone(),
            startup_latency_ms,
            first_callback_latency_ms: None,
        })
    }

    pub fn stop_session(&mut self) -> AudioCaptureStopInfo {
        self.stop_session_with_samples(Vec::new())
    }

    pub fn stop_session_with_samples(&mut self, mut samples: Vec<f32>) -> AudioCaptureStopInfo {
        let pause_error = self.pause_stream().err().map(|error| format!("{error:#}"));
        self.active.store(false, Ordering::Release);
        self.first_callback_latency_ms = self.first_callback_latency_ms();

        let stopped_at = Instant::now();
        self.collect_available_samples(&mut samples);
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
            stale_callback_count: self.stale_callback_count.load(Ordering::Relaxed),
            stale_samples: self.stale_samples.load(Ordering::Relaxed),
        };

        AudioCaptureStopInfo { audio, pause_error }
    }

    pub fn collect_available_samples(&mut self, samples: &mut Vec<f32>) {
        let max_samples = self.max_session_samples();
        let mut dropped_samples = 0_u64;

        while let Ok(sample) = self.consumer.pop() {
            if samples.len() < max_samples {
                samples.push(sample);
            } else {
                dropped_samples += 1;
            }
        }

        if dropped_samples > 0 {
            self.dropped_samples
                .fetch_add(dropped_samples, Ordering::Relaxed);
            self.missed_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn max_session_samples(&self) -> usize {
        max_capture_samples(self.sample_rate, self.channels)
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

    fn drain_samples(&mut self) -> Vec<f32> {
        let available = self.consumer.slots();
        let mut samples = Vec::with_capacity(available);
        while let Ok(sample) = self.consumer.pop() {
            samples.push(sample);
        }
        samples
    }

    fn first_callback_latency_ms(&self) -> Option<u128> {
        let value = self
            .first_callback_latency_ms_atomic
            .load(Ordering::Acquire);
        (value != NO_FIRST_CALLBACK_LATENCY).then_some(u128::from(value))
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

struct AudioCallbackStateAtomics {
    active: Arc<AtomicBool>,
    callback_count: Arc<AtomicU64>,
    dropped_samples: Arc<AtomicU64>,
    missed_chunks: Arc<AtomicU64>,
    stale_callback_count: Arc<AtomicU64>,
    stale_samples: Arc<AtomicU64>,
    session_capture_floor_nanos: Arc<AtomicU64>,
    session_callback_start_nanos: Arc<AtomicU64>,
    first_callback_latency_ms: Arc<AtomicU64>,
}

struct AudioCallbackState {
    producer: Producer<f32>,
    atomics: AudioCallbackStateAtomics,
}

impl AudioCallbackState {
    fn new(producer: Producer<f32>, atomics: AudioCallbackStateAtomics) -> Self {
        Self { producer, atomics }
    }

    fn push_samples<T>(&mut self, samples: &[T], info: &cpal::InputCallbackInfo)
    where
        T: Sample + Copy,
        f32: FromSample<T>,
    {
        if !self.atomics.active.load(Ordering::Acquire) {
            return;
        }

        let timestamp = info.timestamp();
        let capture_nanos = stream_instant_nanos_u64(timestamp.capture);
        let floor_nanos = self
            .atomics
            .session_capture_floor_nanos
            .load(Ordering::Acquire);
        if capture_nanos < floor_nanos {
            self.atomics
                .stale_callback_count
                .fetch_add(1, Ordering::Relaxed);
            self.atomics
                .stale_samples
                .fetch_add(samples.len() as u64, Ordering::Relaxed);
            return;
        }

        self.atomics.callback_count.fetch_add(1, Ordering::Relaxed);
        self.record_first_callback_latency(timestamp.callback);
        let mut dropped_in_callback = 0u64;
        for sample in samples {
            if self.producer.push(f32::from_sample(*sample)).is_err() {
                dropped_in_callback += 1;
            }
        }

        if dropped_in_callback > 0 {
            self.atomics
                .dropped_samples
                .fetch_add(dropped_in_callback, Ordering::Relaxed);
            self.atomics.missed_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_first_callback_latency(&self, callback_at: StreamInstant) {
        let start_nanos = self
            .atomics
            .session_callback_start_nanos
            .load(Ordering::Acquire);
        let callback_nanos = stream_instant_nanos_u64(callback_at);
        let latency_ms = callback_nanos.saturating_sub(start_nanos) / 1_000_000;
        let _ = self.atomics.first_callback_latency_ms.compare_exchange(
            NO_FIRST_CALLBACK_LATENCY,
            latency_ms,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }
}

struct StreamConfigAttempt {
    config: cpal::StreamConfig,
    is_fallback: bool,
}

fn stream_config_attempts(config: &cpal::SupportedStreamConfig) -> Vec<StreamConfigAttempt> {
    let default_config = config.config();
    let mut attempts = Vec::new();

    if let Some(buffer_size) = preferred_fixed_buffer_size(config.buffer_size()) {
        let mut fixed_config = default_config;
        fixed_config.buffer_size = BufferSize::Fixed(buffer_size);
        attempts.push(StreamConfigAttempt {
            config: fixed_config,
            is_fallback: false,
        });
    }

    attempts.push(StreamConfigAttempt {
        config: default_config,
        is_fallback: true,
    });
    attempts
}

fn preferred_fixed_buffer_size(buffer_size: &SupportedBufferSize) -> Option<FrameCount> {
    match buffer_size {
        SupportedBufferSize::Range { min, max } => {
            Some(TARGET_INPUT_BUFFER_FRAMES.clamp(*min, *max))
        }
        SupportedBufferSize::Unknown => None,
    }
}

fn build_stream_for_format(
    sample_format: SampleFormat,
    device: &cpal::Device,
    config: cpal::StreamConfig,
    callback_state: AudioCallbackState,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::I8 => build_stream::<i8>(device, config, callback_state, err_fn),
        SampleFormat::I16 => build_stream::<i16>(device, config, callback_state, err_fn),
        SampleFormat::I32 => build_stream::<i32>(device, config, callback_state, err_fn),
        SampleFormat::U8 => build_stream::<u8>(device, config, callback_state, err_fn),
        SampleFormat::U16 => build_stream::<u16>(device, config, callback_state, err_fn),
        SampleFormat::U32 => build_stream::<u32>(device, config, callback_state, err_fn),
        SampleFormat::F32 => build_stream::<f32>(device, config, callback_state, err_fn),
        SampleFormat::F64 => build_stream::<f64>(device, config, callback_state, err_fn),
        sample_format => anyhow::bail!("unsupported input sample format {sample_format}"),
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
        move |data: &[T], info| {
            callback_state.push_samples(data, info);
        },
        err_fn,
        None,
    )?)
}

fn stream_instant_nanos_u64(value: StreamInstant) -> u64 {
    value.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn max_capture_samples(sample_rate: u32, channels: u16) -> usize {
    (sample_rate as usize)
        .saturating_mul(usize::from(channels))
        .saturating_mul(MAX_CAPTURE_SECONDS)
}

fn callback_buffer_samples(sample_rate: u32, channels: u16) -> usize {
    (sample_rate as usize)
        .saturating_mul(usize::from(channels))
        .saturating_mul(CALLBACK_BUFFER_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::{InputCallbackInfo, InputStreamTimestamp};

    #[test]
    fn audio_callback_state_caps_samples_and_counts_drops() {
        let (producer, mut consumer) = RingBuffer::new(3);
        let active = Arc::new(AtomicBool::new(true));
        let callback_count = Arc::new(AtomicU64::new(0));
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let missed_chunks = Arc::new(AtomicU64::new(0));
        let stale_callback_count = Arc::new(AtomicU64::new(0));
        let stale_samples = Arc::new(AtomicU64::new(0));
        let session_capture_floor_nanos = Arc::new(AtomicU64::new(0));
        let session_callback_start_nanos = Arc::new(AtomicU64::new(0));
        let first_callback_latency_ms = Arc::new(AtomicU64::new(NO_FIRST_CALLBACK_LATENCY));
        let atomics = AudioCallbackStateAtomics {
            active: Arc::clone(&active),
            callback_count: Arc::clone(&callback_count),
            dropped_samples: Arc::clone(&dropped_samples),
            missed_chunks: Arc::clone(&missed_chunks),
            stale_callback_count: Arc::clone(&stale_callback_count),
            stale_samples: Arc::clone(&stale_samples),
            session_capture_floor_nanos: Arc::clone(&session_capture_floor_nanos),
            session_callback_start_nanos: Arc::clone(&session_callback_start_nanos),
            first_callback_latency_ms: Arc::clone(&first_callback_latency_ms),
        };
        let mut state = AudioCallbackState::new(producer, atomics);

        state.push_samples(&[0.1f32, 0.2], &input_info(0, 5));
        state.push_samples(&[0.3f32, 0.4, 0.5], &input_info(10, 15));

        let mut samples = Vec::new();
        while let Ok(sample) = consumer.pop() {
            samples.push(sample);
        }
        assert_eq!(samples, vec![0.1, 0.2, 0.3]);
        assert_eq!(dropped_samples.load(Ordering::Relaxed), 2);
        assert_eq!(missed_chunks.load(Ordering::Relaxed), 1);
        assert_eq!(callback_count.load(Ordering::Relaxed), 2);
        assert_eq!(stale_callback_count.load(Ordering::Relaxed), 0);
        assert_eq!(stale_samples.load(Ordering::Relaxed), 0);
        assert_eq!(first_callback_latency_ms.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn audio_callback_state_discards_stale_callbacks_before_session_floor() {
        let (producer, mut consumer) = RingBuffer::new(8);
        let active = Arc::new(AtomicBool::new(true));
        let callback_count = Arc::new(AtomicU64::new(0));
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let missed_chunks = Arc::new(AtomicU64::new(0));
        let stale_callback_count = Arc::new(AtomicU64::new(0));
        let stale_samples = Arc::new(AtomicU64::new(0));
        let session_capture_floor_nanos = Arc::new(AtomicU64::new(stream_instant_nanos_u64(
            StreamInstant::from_millis(50),
        )));
        let session_callback_start_nanos = Arc::new(AtomicU64::new(stream_instant_nanos_u64(
            StreamInstant::from_millis(50),
        )));
        let first_callback_latency_ms = Arc::new(AtomicU64::new(NO_FIRST_CALLBACK_LATENCY));
        let atomics = AudioCallbackStateAtomics {
            active,
            callback_count: Arc::clone(&callback_count),
            dropped_samples,
            missed_chunks,
            stale_callback_count: Arc::clone(&stale_callback_count),
            stale_samples: Arc::clone(&stale_samples),
            session_capture_floor_nanos,
            session_callback_start_nanos,
            first_callback_latency_ms: Arc::clone(&first_callback_latency_ms),
        };
        let mut state = AudioCallbackState::new(producer, atomics);

        state.push_samples(&[0.1f32, 0.2], &input_info(40, 45));
        state.push_samples(&[0.3f32, 0.4], &input_info(55, 60));

        let mut samples = Vec::new();
        while let Ok(sample) = consumer.pop() {
            samples.push(sample);
        }

        assert_eq!(samples, vec![0.3, 0.4]);
        assert_eq!(callback_count.load(Ordering::Relaxed), 1);
        assert_eq!(stale_callback_count.load(Ordering::Relaxed), 1);
        assert_eq!(stale_samples.load(Ordering::Relaxed), 2);
        assert_eq!(first_callback_latency_ms.load(Ordering::Relaxed), 10);
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
    fn callback_buffer_samples_uses_short_drain_window() {
        assert_eq!(
            callback_buffer_samples(16_000, 1),
            16_000 * CALLBACK_BUFFER_SECONDS
        );
        assert_eq!(
            callback_buffer_samples(48_000, 2),
            48_000 * 2 * CALLBACK_BUFFER_SECONDS
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

    #[test]
    fn preferred_fixed_buffer_size_clamps_to_supported_range() {
        assert_eq!(
            preferred_fixed_buffer_size(&SupportedBufferSize::Range {
                min: 128,
                max: 2048
            }),
            Some(1024)
        );
        assert_eq!(
            preferred_fixed_buffer_size(&SupportedBufferSize::Range {
                min: 2048,
                max: 4096
            }),
            Some(2048)
        );
        assert_eq!(
            preferred_fixed_buffer_size(&SupportedBufferSize::Unknown),
            None
        );
    }

    fn input_info(capture_ms: u64, callback_ms: u64) -> InputCallbackInfo {
        InputCallbackInfo::new(InputStreamTimestamp {
            callback: StreamInstant::from_millis(callback_ms),
            capture: StreamInstant::from_millis(capture_ms),
        })
    }
}
