use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[allow(dead_code)]
pub trait AudioRecorder: Send + Sync {
    fn start(&self) -> Result<()>;
    fn stop(&self) -> Result<Vec<i16>>;
    fn configure_input_device(&self, device_name: Option<&str>) -> Result<()>;
    fn available_input_devices(&self) -> Result<Vec<String>>;
}

#[derive(Default)]
pub struct CpalAudioRecorder {
    state: Mutex<RecorderState>,
}

#[derive(Default)]
struct RecorderState {
    selected_device_name: Option<String>,
    active_recording: Option<ActiveRecording>,
}

struct ActiveRecording {
    stream: Stream,
    buffer: SharedCaptureBuffer,
    sample_rate: u32,
    channels: u16,
}

type SharedCaptureBuffer = Arc<Mutex<Vec<f32>>>;

impl AudioRecorder for CpalAudioRecorder {
    fn start(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("audio recorder state was poisoned");
        if state.active_recording.is_some() {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = input_device(&host, state.selected_device_name.as_deref())?;
        let device_name = device_display_name(&device);
        let supported_config = device
            .default_input_config()
            .with_context(|| format!("failed to read default input config for `{device_name}`"))?;
        let sample_format = supported_config.sample_format();
        let stream_config: StreamConfig = supported_config.into();
        let sample_rate = stream_config.sample_rate;
        let channels = stream_config.channels;
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let stream = build_input_stream(&device, &stream_config, sample_format, Arc::clone(&buffer))
            .with_context(|| {
                format!(
                    "failed to build input stream for `{device_name}` ({sample_rate} Hz, {channels} channels, {sample_format:?})"
                )
            })?;

        stream
            .play()
            .with_context(|| format!("failed to start input stream for `{device_name}`"))?;
        if voice_debug_enabled() {
            eprintln!(
                "Started microphone capture from `{device_name}` ({sample_rate} Hz, {channels} channels, {sample_format:?})."
            );
        }

        state.active_recording = Some(ActiveRecording {
            stream,
            buffer,
            sample_rate,
            channels,
        });
        Ok(())
    }

    fn stop(&self) -> Result<Vec<i16>> {
        let active_recording = self
            .state
            .lock()
            .expect("audio recorder state was poisoned")
            .active_recording
            .take();
        let Some(active_recording) = active_recording else {
            return Ok(Vec::new());
        };

        drop(active_recording.stream);
        let samples = active_recording
            .buffer
            .lock()
            .expect("audio capture buffer was poisoned")
            .clone();
        Ok(downmix_and_resample_to_i16(
            &samples,
            active_recording.channels,
            active_recording.sample_rate,
        ))
    }

    fn configure_input_device(&self, device_name: Option<&str>) -> Result<()> {
        let selected_device_name = normalize_device_name(device_name);
        if let Some(device_name) = selected_device_name.as_deref() {
            let devices = self.available_input_devices()?;
            if !devices.iter().any(|available| available == device_name) {
                bail!("microphone device is not available: {device_name}");
            }
        }

        let mut state = self
            .state
            .lock()
            .expect("audio recorder state was poisoned");
        if state.active_recording.is_some() {
            bail!("cannot change microphone while recording");
        }
        state.selected_device_name = selected_device_name;
        Ok(())
    }

    fn available_input_devices(&self) -> Result<Vec<String>> {
        let host = cpal::default_host();
        let mut names = Vec::new();
        for device in host
            .input_devices()
            .context("failed to enumerate input devices")?
        {
            if let Ok(name) = device
                .description()
                .map(|description| description.name().to_string())
                && !name.trim().is_empty()
                && !names.iter().any(|existing| existing == &name)
            {
                names.push(name);
            }
        }
        Ok(names)
    }
}

#[derive(Debug, Default)]
pub struct StubAudioRecorder {
    selected_device_name: Mutex<Option<String>>,
}

impl AudioRecorder for StubAudioRecorder {
    fn start(&self) -> Result<()> {
        Ok(())
    }

    fn stop(&self) -> Result<Vec<i16>> {
        Ok(Vec::new())
    }

    fn configure_input_device(&self, device_name: Option<&str>) -> Result<()> {
        *self
            .selected_device_name
            .lock()
            .expect("stub audio state was poisoned") = normalize_device_name(device_name);
        Ok(())
    }

    fn available_input_devices(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

fn input_device(host: &cpal::Host, selected_device_name: Option<&str>) -> Result<cpal::Device> {
    if let Some(selected_device_name) = selected_device_name {
        for device in host
            .input_devices()
            .context("failed to enumerate input devices")?
        {
            if device_display_name(&device) == selected_device_name {
                return Ok(device);
            }
        }
        bail!("microphone device is not available: {selected_device_name}");
    }

    host.default_input_device()
        .ok_or_else(|| anyhow!("no default input device available"))
}

fn device_display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "Unknown input".to_string())
}

fn build_input_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    buffer: SharedCaptureBuffer,
) -> Result<Stream> {
    let err_fn = |error| eprintln!("Microphone input stream error: {error}");
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: f32| sample.clamp(-1.0, 1.0)),
            err_fn,
            None,
        )?,
        SampleFormat::F64 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: f64| (sample as f32).clamp(-1.0, 1.0)),
            err_fn,
            None,
        )?,
        SampleFormat::I8 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: i8| sample as f32 / i8::MAX as f32),
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: i16| sample as f32 / i16::MAX as f32),
            err_fn,
            None,
        )?,
        SampleFormat::I32 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: i32| sample as f32 / i32::MAX as f32),
            err_fn,
            None,
        )?,
        SampleFormat::U8 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: u8| (sample as f32 - 128.0) / 128.0),
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: u16| (sample as f32 - 32_768.0) / 32_768.0),
            err_fn,
            None,
        )?,
        SampleFormat::U32 => device.build_input_stream(
            config,
            capture_callback(buffer, |sample: u32| {
                (sample as f32 - 2_147_483_648.0) / 2_147_483_648.0
            }),
            err_fn,
            None,
        )?,
        other => bail!("unsupported microphone sample format: {other:?}"),
    };

    Ok(stream)
}

fn capture_callback<T>(
    buffer: SharedCaptureBuffer,
    convert_sample: impl Fn(T) -> f32 + Send + 'static,
) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    T: Copy + Send + 'static,
{
    move |data, _| {
        if let Ok(mut buffer) = buffer.try_lock() {
            buffer.reserve(data.len());
            buffer.extend(data.iter().map(|sample| convert_sample(*sample)));
        }
    }
}

fn normalize_device_name(device_name: Option<&str>) -> Option<String> {
    device_name
        .map(str::trim)
        .filter(|device_name| !device_name.is_empty())
        .map(ToOwned::to_owned)
}

fn voice_debug_enabled() -> bool {
    std::env::var_os("VOICE_DEBUG").is_some()
}

pub(crate) fn downmix_and_resample_to_i16(
    interleaved_samples: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Vec<i16> {
    if interleaved_samples.is_empty() || channels == 0 || sample_rate == 0 {
        return Vec::new();
    }

    let channels = channels as usize;
    let mono = interleaved_samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
        .collect::<Vec<_>>();
    let resampled = resample_linear(&mono, sample_rate, WHISPER_SAMPLE_RATE);
    resampled
        .into_iter()
        .map(|sample| {
            let sample = sample.clamp(-1.0, 1.0);
            (sample * i16::MAX as f32).round() as i16
        })
        .collect()
}

fn resample_linear(samples: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    if input_rate == output_rate {
        return samples.to_vec();
    }

    let output_len =
        ((samples.len() as f64) * output_rate as f64 / input_rate as f64).round() as usize;
    if output_len == 0 {
        return Vec::new();
    }

    let ratio = input_rate as f64 / output_rate as f64;
    (0..output_len)
        .map(|index| {
            let position = index as f64 * ratio;
            let left_index = position.floor() as usize;
            let right_index = (left_index + 1).min(samples.len() - 1);
            let fraction = (position - left_index as f64) as f32;
            let left = samples[left_index];
            let right = samples[right_index];
            left + (right - left) * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_samples_stay_mono() {
        let samples = downmix_and_resample_to_i16(&[0.0, 0.5, -0.5], 1, WHISPER_SAMPLE_RATE);

        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], 0);
        assert!(samples[1] > 16_000);
        assert!(samples[2] < -16_000);
    }

    #[test]
    fn stereo_samples_are_downmixed() {
        let samples = downmix_and_resample_to_i16(&[1.0, -1.0, 0.5, 0.5], 2, WHISPER_SAMPLE_RATE);

        assert_eq!(samples, vec![0, 16_384]);
    }

    #[test]
    fn resamples_48khz_to_16khz() {
        let input = vec![0.25; 48_000];

        let samples = downmix_and_resample_to_i16(&input, 1, 48_000);

        assert_eq!(samples.len(), 16_000);
        assert!(samples.iter().all(|sample| *sample > 8_000));
    }

    #[test]
    fn empty_buffer_remains_empty() {
        assert!(downmix_and_resample_to_i16(&[], 1, 48_000).is_empty());
        assert!(downmix_and_resample_to_i16(&[1.0], 0, 48_000).is_empty());
        assert!(downmix_and_resample_to_i16(&[1.0], 1, 0).is_empty());
    }
}
