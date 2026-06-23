use std::f32::consts::TAU;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream};
use shared::{MAX_BEEP_VOLUME_PERCENT, MIN_BEEP_VOLUME_PERCENT};
use tracing::{debug, warn};

use crate::command::AppCommand;

const LOW_TONE_HZ: f32 = 440.0;
const HIGH_TONE_HZ: f32 = 880.0;
const TONE_DURATION: Duration = Duration::from_millis(90);
const GAP_DURATION: Duration = Duration::from_millis(35);
const FADE_DURATION: Duration = Duration::from_millis(8);
const MAX_AMPLITUDE: f32 = 0.18;
const PLAYBACK_TIMEOUT_PADDING: Duration = Duration::from_secs(1);

pub struct BeepService {
    worker_tx: mpsc::Sender<BeepCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeepCue {
    Start,
    Stop,
}

impl BeepService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("quillspeak-beep".to_string())
            .spawn(move || beep_worker_loop(worker_rx, command_tx))
            .map_err(|error| anyhow!("failed to spawn beep worker: {error}"))?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn play_start_cue(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        volume_percent: u8,
    ) -> Result<()> {
        self.worker_tx
            .send(BeepCommand::Play {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
                cue: BeepCue::Start,
                volume_percent,
                notify_start_completion: true,
            })
            .map_err(|_| anyhow!("beep worker is not running"))
    }

    pub fn play_stop_cue(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        volume_percent: u8,
    ) -> Result<()> {
        self.worker_tx
            .send(BeepCommand::Play {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
                cue: BeepCue::Stop,
                volume_percent,
                notify_start_completion: false,
            })
            .map_err(|_| anyhow!("beep worker is not running"))
    }

    pub fn shutdown(mut self) {
        let _ = self.worker_tx.send(BeepCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "beep worker panicked during shutdown");
        }
    }
}

enum BeepCommand {
    Play {
        recording_id: u64,
        shortcut_id: String,
        cue: BeepCue,
        volume_percent: u8,
        notify_start_completion: bool,
    },
    Shutdown,
}

fn beep_worker_loop(worker_rx: mpsc::Receiver<BeepCommand>, command_tx: mpsc::Sender<AppCommand>) {
    for command in worker_rx {
        match command {
            BeepCommand::Play {
                recording_id,
                shortcut_id,
                cue,
                volume_percent,
                notify_start_completion,
            } => {
                let result = play_cue(cue, volume_percent).map_err(|error| format!("{error:#}"));
                if notify_start_completion {
                    let _ = command_tx.send(AppCommand::RecordingStartCueFinished {
                        recording_id,
                        shortcut_id,
                        result,
                    });
                } else if let Err(error) = result {
                    warn!(
                        recording_id,
                        shortcut_id, error, "failed to play recording cue"
                    );
                }
            }
            BeepCommand::Shutdown => break,
        }
    }
}

fn play_cue(cue: BeepCue, volume_percent: u8) -> Result<()> {
    let (device, device_label) = resolve_default_output_device()?;
    let supported_config = device
        .default_output_config()
        .with_context(|| format!("failed to read default output config for {device_label}"))?;
    let sample_rate = supported_config.sample_rate();
    let channels = supported_config.channels();
    anyhow::ensure!(sample_rate > 0, "output device reported zero sample rate");
    anyhow::ensure!(channels > 0, "output device reported zero channels");

    let samples = cue_samples(cue, sample_rate, channels, volume_percent);
    let playback_duration = playback_duration_for_samples(samples.len(), sample_rate, channels);
    let (done_tx, done_rx) = mpsc::channel();
    let err_label = device_label.clone();
    let err_fn = move |error| warn!(?error, output = %err_label, "beep output stream error");
    let stream = build_output_stream(
        supported_config.sample_format(),
        &device,
        supported_config.config(),
        samples,
        done_tx,
        err_fn,
    )?;
    stream
        .play()
        .with_context(|| format!("failed to start output stream for {device_label}"))?;
    done_rx
        .recv_timeout(playback_duration + PLAYBACK_TIMEOUT_PADDING)
        .context("timed out waiting for recording cue playback")?;
    drop(stream);
    debug!(cue = ?cue, output = %device_label, "recording cue played");
    Ok(())
}

fn resolve_default_output_device() -> Result<(cpal::Device, String)> {
    let mut fallback_error = None;
    for host_id in preferred_host_ids() {
        let host = match cpal::host_from_id(host_id) {
            Ok(host) => host,
            Err(error) => {
                fallback_error = Some(format!("{error}"));
                continue;
            }
        };
        if let Some(device) = host.default_output_device() {
            return Ok((device, format!("System Default Output ({host_id})")));
        }
    }

    if let Some(error) = fallback_error {
        warn!(
            error,
            "no default output device found on available audio hosts"
        );
    }
    bail!("no default audio output device is available");
}

fn preferred_host_ids() -> Vec<cpal::HostId> {
    let available = cpal::available_hosts();
    let mut ordered = Vec::new();

    for preferred in ["pipewire", "pulseaudio", "alsa"] {
        if let Some(host_id) = available
            .iter()
            .copied()
            .find(|host_id| host_id.to_string() == preferred)
        {
            ordered.push(host_id);
        }
    }

    for host_id in available {
        if !ordered.contains(&host_id) {
            ordered.push(host_id);
        }
    }
    ordered
}

fn build_output_stream(
    sample_format: SampleFormat,
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Vec<f32>,
    done_tx: mpsc::Sender<()>,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::I8 => build_stream::<i8>(device, config, samples, done_tx, err_fn),
        SampleFormat::I16 => build_stream::<i16>(device, config, samples, done_tx, err_fn),
        SampleFormat::I32 => build_stream::<i32>(device, config, samples, done_tx, err_fn),
        SampleFormat::U8 => build_stream::<u8>(device, config, samples, done_tx, err_fn),
        SampleFormat::U16 => build_stream::<u16>(device, config, samples, done_tx, err_fn),
        SampleFormat::U32 => build_stream::<u32>(device, config, samples, done_tx, err_fn),
        SampleFormat::F32 => build_stream::<f32>(device, config, samples, done_tx, err_fn),
        SampleFormat::F64 => build_stream::<f64>(device, config, samples, done_tx, err_fn),
        sample_format => bail!("unsupported output sample format {sample_format}"),
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Vec<f32>,
    done_tx: mpsc::Sender<()>,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream>
where
    T: Sample + cpal::SizedSample + FromSample<f32>,
{
    let mut index = 0usize;
    let mut done_tx = Some(done_tx);
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            for sample in data {
                let value = samples.get(index).copied().unwrap_or(0.0);
                *sample = T::from_sample(value);
                index = index.saturating_add(1);
            }

            if index >= samples.len()
                && let Some(done_tx) = done_tx.take()
            {
                let _ = done_tx.send(());
            }
        },
        err_fn,
        None,
    )?)
}

fn cue_samples(cue: BeepCue, sample_rate: u32, channels: u16, volume_percent: u8) -> Vec<f32> {
    let tones = cue_tones(cue);
    let amplitude = amplitude_for_volume_percent(volume_percent);
    let mut samples = Vec::new();
    append_tone(&mut samples, tones[0], sample_rate, channels, amplitude);
    append_silence(&mut samples, sample_rate, channels);
    append_tone(&mut samples, tones[1], sample_rate, channels, amplitude);
    samples
}

fn amplitude_for_volume_percent(volume_percent: u8) -> f32 {
    let volume_percent = volume_percent.clamp(MIN_BEEP_VOLUME_PERCENT, MAX_BEEP_VOLUME_PERCENT);
    MAX_AMPLITUDE * f32::from(volume_percent) / f32::from(MAX_BEEP_VOLUME_PERCENT)
}

fn cue_tones(cue: BeepCue) -> [f32; 2] {
    match cue {
        BeepCue::Start => [LOW_TONE_HZ, HIGH_TONE_HZ],
        BeepCue::Stop => [HIGH_TONE_HZ, LOW_TONE_HZ],
    }
}

fn append_tone(
    samples: &mut Vec<f32>,
    frequency_hz: f32,
    sample_rate: u32,
    channels: u16,
    amplitude: f32,
) {
    let frames = duration_frames(TONE_DURATION, sample_rate);
    let fade_frames = duration_frames(FADE_DURATION, sample_rate).max(1);
    for frame in 0..frames {
        let phase = TAU * frequency_hz * frame as f32 / sample_rate as f32;
        let envelope = fade_envelope(frame, frames, fade_frames);
        let value = phase.sin() * amplitude * envelope;
        for _ in 0..channels {
            samples.push(value);
        }
    }
}

fn append_silence(samples: &mut Vec<f32>, sample_rate: u32, channels: u16) {
    let frames = duration_frames(GAP_DURATION, sample_rate);
    samples.extend(std::iter::repeat_n(
        0.0,
        frames.saturating_mul(usize::from(channels)),
    ));
}

fn fade_envelope(frame: usize, frames: usize, fade_frames: usize) -> f32 {
    if frame < fade_frames {
        return frame as f32 / fade_frames as f32;
    }
    let remaining = frames.saturating_sub(frame + 1);
    if remaining < fade_frames {
        return remaining as f32 / fade_frames as f32;
    }
    1.0
}

fn duration_frames(duration: Duration, sample_rate: u32) -> usize {
    duration
        .as_nanos()
        .saturating_mul(u128::from(sample_rate))
        .checked_div(1_000_000_000)
        .unwrap_or(0) as usize
}

fn playback_duration_for_samples(samples: usize, sample_rate: u32, channels: u16) -> Duration {
    let frames = samples / usize::from(channels);
    Duration::from_secs_f64(frames as f64 / f64::from(sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_tones_match_start_and_stop_order() {
        assert_eq!(cue_tones(BeepCue::Start), [LOW_TONE_HZ, HIGH_TONE_HZ]);
        assert_eq!(cue_tones(BeepCue::Stop), [HIGH_TONE_HZ, LOW_TONE_HZ]);
    }

    #[test]
    fn cue_samples_are_interleaved_for_all_channels() {
        let samples = cue_samples(BeepCue::Start, 1_000, 2, 100);

        assert_eq!(samples.len() % 2, 0);
        for frame in samples.chunks_exact(2) {
            assert_eq!(frame[0], frame[1]);
        }
    }

    #[test]
    fn cue_samples_scale_with_volume_percent() {
        let full_volume = cue_samples(BeepCue::Start, 8_000, 1, 100)
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max);
        let half_volume = cue_samples(BeepCue::Start, 8_000, 1, 50)
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max);

        assert!(full_volume > 0.0);
        assert!((half_volume - full_volume * 0.5).abs() < 0.001);
    }
}
