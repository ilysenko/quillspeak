use std::sync::mpsc;

use ksni::blocking::{Handle as KsniHandle, TrayMethods as KsniTrayMethods};

use crate::command::AppCommand;
use crate::recording::RecordingPhase;

const TRAY_ICON_SIZE: i32 = 22;
const TRAY_IDLE_COLOR: [u8; 4] = [255, 255, 255, 255];
const TRAY_RECORDING_COLOR: [u8; 4] = [239, 68, 68, 255];
const TRAY_PROCESSING_COLOR: [u8; 4] = [245, 158, 11, 255];

pub struct Tray {
    handle: KsniHandle<MyAppTray>,
}

impl Tray {
    pub fn new(command_tx: mpsc::Sender<AppCommand>) -> Result<Self, ksni::Error> {
        let tray = MyAppTray {
            command_tx,
            recording_phase: RecordingPhase::Idle,
        };
        let handle = tray.spawn()?;
        Ok(Self { handle })
    }

    pub fn set_recording_phase(&self, phase: RecordingPhase) {
        let _ = self.handle.update(|tray| {
            tray.recording_phase = phase;
        });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}

struct MyAppTray {
    command_tx: mpsc::Sender<AppCommand>,
    recording_phase: RecordingPhase,
}

impl MyAppTray {
    fn send(&self, command: AppCommand) {
        let _ = self.command_tx.send(command);
    }
}

impl ksni::Tray for MyAppTray {
    fn id(&self) -> String {
        "myapp".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn title(&self) -> String {
        match self.recording_phase {
            RecordingPhase::Idle => "MyApp".to_string(),
            RecordingPhase::Arming => "MyApp - Recording".to_string(),
            RecordingPhase::Recording => "MyApp - Recording".to_string(),
            RecordingPhase::Processing => "MyApp - Processing".to_string(),
        }
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon_pixmap(icon_color(self.recording_phase))]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(AppCommand::ShowSettings);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let recording_item = recording_menu_item(self.recording_phase);
        vec![
            StandardItem {
                label: "Show Settings".to_string(),
                icon_name: "preferences-system".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(AppCommand::ShowSettings)),
                ..Default::default()
            }
            .into(),
            recording_item.into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                icon_name: "application-exit".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(AppCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn recording_menu_item(phase: RecordingPhase) -> ksni::menu::StandardItem<MyAppTray> {
    let (label, icon_name, enabled) = match phase {
        RecordingPhase::Idle => ("Start Recording", "media-record", true),
        RecordingPhase::Arming => ("Stop Recording", "media-playback-stop", true),
        RecordingPhase::Recording => ("Stop Recording", "media-playback-stop", true),
        RecordingPhase::Processing => ("Processing...", "media-playback-pause", false),
    };

    ksni::menu::StandardItem {
        label: label.to_string(),
        icon_name: icon_name.to_string(),
        enabled,
        activate: Box::new(|tray: &mut MyAppTray| tray.send(AppCommand::ToggleRecording)),
        ..Default::default()
    }
}

fn icon_color(phase: RecordingPhase) -> [u8; 4] {
    match phase {
        RecordingPhase::Idle => TRAY_IDLE_COLOR,
        RecordingPhase::Arming => TRAY_RECORDING_COLOR,
        RecordingPhase::Recording => TRAY_RECORDING_COLOR,
        RecordingPhase::Processing => TRAY_PROCESSING_COLOR,
    }
}

fn tray_icon_pixmap(color: [u8; 4]) -> ksni::Icon {
    let mut data = vec![0; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];

    for y in 0..TRAY_ICON_SIZE {
        for x in 0..TRAY_ICON_SIZE {
            if is_microphone_pixel(x, y) {
                write_argb_pixel(&mut data, x, y, color);
            }
        }
    }

    ksni::Icon {
        width: TRAY_ICON_SIZE,
        height: TRAY_ICON_SIZE,
        data,
    }
}

fn is_microphone_pixel(x: i32, y: i32) -> bool {
    let body = match y {
        4 | 13 => (8..=13).contains(&x),
        5..=12 => (7..=14).contains(&x),
        _ => false,
    };
    let side = ((5..=16).contains(&y) && (x == 5 || x == 16)) || (y == 16 && (6..=15).contains(&x));
    let stem = (17..=19).contains(&y) && (x == 10 || x == 11);
    let base = y == 20 && (7..=14).contains(&x);

    body || side || stem || base
}

fn write_argb_pixel(data: &mut [u8], x: i32, y: i32, color: [u8; 4]) {
    let offset = ((y * TRAY_ICON_SIZE + x) * 4) as usize;
    let [red, green, blue, alpha] = color;

    data[offset] = alpha;
    data[offset + 1] = red;
    data[offset + 2] = green;
    data[offset + 3] = blue;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_color_tracks_recording_phase() {
        assert_eq!(icon_color(RecordingPhase::Idle), TRAY_IDLE_COLOR);
        assert_eq!(icon_color(RecordingPhase::Arming), TRAY_RECORDING_COLOR);
        assert_eq!(icon_color(RecordingPhase::Recording), TRAY_RECORDING_COLOR);
        assert_eq!(
            icon_color(RecordingPhase::Processing),
            TRAY_PROCESSING_COLOR
        );
    }

    #[test]
    fn tray_icon_pixmap_writes_argb_pixels() {
        let icon = tray_icon_pixmap(TRAY_RECORDING_COLOR);
        let offset = ((5 * TRAY_ICON_SIZE + 8) * 4) as usize;

        assert_eq!(icon.data[offset], TRAY_RECORDING_COLOR[3]);
        assert_eq!(icon.data[offset + 1], TRAY_RECORDING_COLOR[0]);
        assert_eq!(icon.data[offset + 2], TRAY_RECORDING_COLOR[1]);
        assert_eq!(icon.data[offset + 3], TRAY_RECORDING_COLOR[2]);
    }
}
