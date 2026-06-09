use std::collections::HashSet;
use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait};
use shared::AudioInputRef;
use tracing::{debug, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInputDevice {
    pub reference: AudioInputRef,
    pub label: String,
}

pub fn list_input_devices() -> Vec<AudioInputDevice> {
    let mut devices = vec![AudioInputDevice {
        reference: AudioInputRef::SystemDefault,
        label: "System Default".to_string(),
    }];
    let mut seen = HashSet::from([AudioInputRef::SystemDefault.stable_key()]);

    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            debug!(host = %host_id, "audio host is not available");
            continue;
        };
        let default_id = host
            .default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());

        let input_devices = match host.input_devices() {
            Ok(devices) => devices,
            Err(error) => {
                debug!(host = %host_id, ?error, "failed to enumerate input devices");
                continue;
            }
        };

        for device in input_devices {
            let Ok(device_id) = device.id() else {
                debug!(host = %host_id, device = %device, "input device has no stable id");
                continue;
            };
            let device_key = device_id.to_string();
            let id = device_id.id().to_string();
            let host = host_id.to_string();
            let is_default = default_id.as_deref() == Some(device_key.as_str());
            let name = device.to_string();
            let label = if is_default {
                format!("{name} ({host}, default)")
            } else {
                format!("{name} ({host})")
            };
            let reference = AudioInputRef::Device {
                host,
                id,
                label: label.clone(),
            };

            if seen.insert(reference.stable_key()) {
                devices.push(AudioInputDevice { reference, label });
            }
        }
    }

    devices
}

pub fn resolve_input_device(input: &AudioInputRef) -> anyhow::Result<(cpal::Device, String)> {
    match input {
        AudioInputRef::SystemDefault => resolve_default_input_device(),
        AudioInputRef::Device { host, id, label } => {
            let host_id = cpal::HostId::from_str(host)
                .map_err(|error| anyhow::anyhow!("unsupported audio host {host}: {error}"))?;
            let host = cpal::host_from_id(host_id)
                .map_err(|error| anyhow::anyhow!("audio host {host} is unavailable: {error}"))?;
            let device_id = cpal::DeviceId::new(host_id, id);
            let device = host
                .device_by_id(&device_id)
                .ok_or_else(|| anyhow::anyhow!("audio input device is unavailable: {label}"))?;
            Ok((device, label.clone()))
        }
    }
}

fn resolve_default_input_device() -> anyhow::Result<(cpal::Device, String)> {
    let mut fallback_error = None;
    for host_id in preferred_host_ids() {
        let host = match cpal::host_from_id(host_id) {
            Ok(host) => host,
            Err(error) => {
                fallback_error = Some(format!("{error}"));
                continue;
            }
        };
        if let Some(device) = host.default_input_device() {
            return Ok((device, format!("System Default ({host_id})")));
        }
    }

    if let Some(error) = fallback_error {
        warn!(
            error,
            "no default input device found on available audio hosts"
        );
    }
    anyhow::bail!("no default audio input device is available");
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
