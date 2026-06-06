use std::sync::mpsc;

use ksni::blocking::{Handle as KsniHandle, TrayMethods as KsniTrayMethods};

use crate::command::AppCommand;

const TRAY_ICON_SIZE: i32 = 22;

pub struct Tray {
    handle: KsniHandle<MyAppTray>,
}

impl Tray {
    pub fn new(command_tx: mpsc::Sender<AppCommand>) -> Result<Self, ksni::Error> {
        let tray = MyAppTray { command_tx };
        let handle = tray.spawn()?;
        Ok(Self { handle })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}

struct MyAppTray {
    command_tx: mpsc::Sender<AppCommand>,
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
        "MyApp".to_string()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon_pixmap()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(AppCommand::ShowSettings);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        vec![
            StandardItem {
                label: "Show Settings".to_string(),
                icon_name: "preferences-system".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(AppCommand::ShowSettings)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start Recording".to_string(),
                icon_name: "media-record".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(AppCommand::StartRecording)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop Recording".to_string(),
                icon_name: "media-playback-stop".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(AppCommand::StopRecording)),
                ..Default::default()
            }
            .into(),
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

fn tray_icon_pixmap() -> ksni::Icon {
    let mut data = vec![0; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];
    let color = [255, 255, 255, 255];

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
