use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use appindicator3::{Indicator, IndicatorCategory, IndicatorStatus, prelude::*};
use directories::BaseDirs;
use gtk::prelude::*;
use ksni::blocking::{Handle as KsniHandle, TrayMethods as KsniTrayMethods};

use crate::activity::VoiceActivityState;

const TRAY_ICON_SIZE: i32 = 22;
const APP_INDICATOR_ICONS: &[(&str, &str)] = &[
    (
        "voice-idle.svg",
        include_str!("../assets/icons/voice-idle.svg"),
    ),
    (
        "voice-recording.svg",
        include_str!("../assets/icons/voice-recording.svg"),
    ),
    (
        "voice-processing.svg",
        include_str!("../assets/icons/voice-processing.svg"),
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    Quit,
    StatusNotifierOffline,
    StatusNotifierOnline,
}

pub trait TrayBackend {
    fn backend_name(&self) -> &'static str;
    fn set_visual_state(&self, state: VoiceActivityState);
}

pub fn create_tray_backend(
    sender: gtk::glib::Sender<TrayAction>,
    initial_state: VoiceActivityState,
) -> Box<dyn TrayBackend> {
    match StatusNotifierTray::new(sender.clone(), initial_state) {
        Ok(tray) => {
            eprintln!("Using StatusNotifierItem tray backend.");
            Box::new(tray)
        }
        Err(error) => {
            eprintln!(
                "StatusNotifierItem tray unavailable: {error}. Falling back to AppIndicator."
            );
            create_fallback_backend(sender, initial_state)
        }
    }
}

fn create_fallback_backend(
    sender: gtk::glib::Sender<TrayAction>,
    initial_state: VoiceActivityState,
) -> Box<dyn TrayBackend> {
    match choose_backend_kind(false, true) {
        TrayBackendKind::AppIndicator => match AppIndicatorTray::new(sender.clone(), initial_state)
        {
            Ok(tray) => {
                eprintln!("Using legacy AppIndicator tray backend.");
                Box::new(tray)
            }
            Err(error) => Box::new(NoTrayFallback::new(
                sender,
                format!("legacy AppIndicator fallback failed: {error:#}"),
            )),
        },
        TrayBackendKind::None => Box::new(NoTrayFallback::new(
            sender,
            "no supported tray backend is available".to_string(),
        )),
        TrayBackendKind::StatusNotifier => unreachable!("primary SNI backend already failed"),
    }
}

pub struct ManagedTrayBackend {
    primary: Box<dyn TrayBackend>,
    legacy_fallback: RefCell<Option<Box<dyn TrayBackend>>>,
    visual_state: RefCell<VoiceActivityState>,
}

impl ManagedTrayBackend {
    pub fn new(primary: Box<dyn TrayBackend>, initial_state: VoiceActivityState) -> Self {
        primary.set_visual_state(initial_state);

        Self {
            primary,
            legacy_fallback: RefCell::new(None),
            visual_state: RefCell::new(initial_state),
        }
    }

    pub fn visual_state(&self) -> VoiceActivityState {
        *self.visual_state.borrow()
    }

    pub fn start_legacy_fallback(&self, fallback: Box<dyn TrayBackend>) {
        fallback.set_visual_state(self.visual_state());
        self.legacy_fallback.replace(Some(fallback));
    }

    pub fn stop_legacy_fallback(&self) -> bool {
        self.legacy_fallback.borrow_mut().take().is_some()
    }

    pub fn has_legacy_fallback(&self) -> bool {
        self.legacy_fallback.borrow().is_some()
    }
}

impl TrayBackend for ManagedTrayBackend {
    fn backend_name(&self) -> &'static str {
        self.primary.backend_name()
    }

    fn set_visual_state(&self, state: VoiceActivityState) {
        self.visual_state.replace(state);
        self.primary.set_visual_state(state);
        if let Some(fallback) = self.legacy_fallback.borrow().as_ref() {
            fallback.set_visual_state(state);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayBackendKind {
    StatusNotifier,
    AppIndicator,
    None,
}

fn choose_backend_kind(sni_available: bool, legacy_available: bool) -> TrayBackendKind {
    if sni_available {
        TrayBackendKind::StatusNotifier
    } else if legacy_available {
        TrayBackendKind::AppIndicator
    } else {
        TrayBackendKind::None
    }
}

pub struct StatusNotifierTray {
    handle: KsniHandle<VoiceStatusNotifierTray>,
}

impl StatusNotifierTray {
    pub fn new(
        sender: gtk::glib::Sender<TrayAction>,
        initial_state: VoiceActivityState,
    ) -> std::result::Result<Self, ksni::Error> {
        let tray = VoiceStatusNotifierTray {
            sender,
            visual_state: initial_state,
        };
        let handle = tray.spawn()?;

        Ok(Self { handle })
    }
}

impl TrayBackend for StatusNotifierTray {
    fn backend_name(&self) -> &'static str {
        "status-notifier-item"
    }

    fn set_visual_state(&self, state: VoiceActivityState) {
        let _ = self.handle.update(|tray| {
            tray.visual_state = state;
        });
    }
}

impl Drop for StatusNotifierTray {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}

struct VoiceStatusNotifierTray {
    sender: gtk::glib::Sender<TrayAction>,
    visual_state: VoiceActivityState,
}

impl VoiceStatusNotifierTray {
    fn send(&self, action: TrayAction) {
        let _ = self.sender.send(action);
    }
}

impl ksni::Tray for VoiceStatusNotifierTray {
    fn id(&self) -> String {
        "voice".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn title(&self) -> String {
        "Voice".to_string()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon_pixmap(self.visual_state)]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::OpenSettings);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        vec![
            StandardItem {
                label: "Settings".to_string(),
                icon_name: "preferences-system".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::OpenSettings)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                icon_name: "application-exit".to_string(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.send(TrayAction::StatusNotifierOnline);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("StatusNotifier watcher is offline: {reason:?}");
        self.send(TrayAction::StatusNotifierOffline);
        true
    }
}

pub struct AppIndicatorTray {
    indicator: appindicator3::Indicator,
    _menu: gtk::Menu,
}

impl AppIndicatorTray {
    pub fn new(
        sender: gtk::glib::Sender<TrayAction>,
        initial_state: VoiceActivityState,
    ) -> Result<Self> {
        let icon_theme_path = ensure_appindicator_icons()?;
        let icon_theme_path = icon_theme_path
            .to_str()
            .context("appindicator icon path is not valid UTF-8")?;
        let menu = gtk::Menu::new();

        let settings_item = gtk::MenuItem::with_label("Settings");
        let settings_action = sender.clone();
        settings_item.connect_activate(move |_| {
            let _ = settings_action.send(TrayAction::OpenSettings);
        });
        menu.append(&settings_item);

        let separator = gtk::SeparatorMenuItem::new();
        menu.append(&separator);

        let quit_item = gtk::MenuItem::with_label("Quit");
        quit_item.connect_activate(move |_| {
            let _ = sender.send(TrayAction::Quit);
        });
        menu.append(&quit_item);
        menu.show_all();

        let indicator = Indicator::with_path(
            "voice",
            icon_name_for_state(initial_state),
            IndicatorCategory::ApplicationStatus,
            icon_theme_path,
        );
        indicator.set_title(Some("Voice"));
        indicator.set_icon_theme_path(icon_theme_path);
        indicator.set_icon_full(icon_name_for_state(initial_state), "Voice");
        indicator.set_menu(Some(&menu));
        indicator.set_status(IndicatorStatus::Active);

        Ok(Self {
            indicator,
            _menu: menu,
        })
    }
}

impl TrayBackend for AppIndicatorTray {
    fn backend_name(&self) -> &'static str {
        "appindicator3-fallback"
    }

    fn set_visual_state(&self, state: VoiceActivityState) {
        self.indicator.set_icon_full(
            icon_name_for_state(state),
            icon_description_for_state(state),
        );
    }
}

pub struct NoTrayFallback {
    _reason: String,
}

impl NoTrayFallback {
    fn new(sender: gtk::glib::Sender<TrayAction>, reason: String) -> Self {
        eprintln!("No tray backend is available: {reason}");
        let _ = sender.send(TrayAction::OpenSettings);
        Self { _reason: reason }
    }
}

impl TrayBackend for NoTrayFallback {
    fn backend_name(&self) -> &'static str {
        "no-tray"
    }

    fn set_visual_state(&self, _state: VoiceActivityState) {}
}

fn icon_name_for_state(state: VoiceActivityState) -> &'static str {
    match state {
        VoiceActivityState::Idle => "voice-idle",
        VoiceActivityState::Recording => "voice-recording",
        VoiceActivityState::Processing => "voice-processing",
    }
}

fn icon_description_for_state(state: VoiceActivityState) -> &'static str {
    match state {
        VoiceActivityState::Idle => "Voice ready",
        VoiceActivityState::Recording => "Voice recording",
        VoiceActivityState::Processing => "Voice processing",
    }
}

fn color_for_state(state: VoiceActivityState) -> [u8; 4] {
    match state {
        VoiceActivityState::Idle => [255, 255, 255, 255],
        VoiceActivityState::Recording => [255, 80, 80, 255],
        VoiceActivityState::Processing => [255, 214, 88, 255],
    }
}

fn tray_icon_pixmap(state: VoiceActivityState) -> ksni::Icon {
    let mut data = vec![0; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];
    let color = color_for_state(state);

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

fn ensure_appindicator_icons() -> Result<PathBuf> {
    let icon_dir = BaseDirs::new()
        .map(|base_dirs| base_dirs.cache_dir().join("voice/icons"))
        .unwrap_or_else(|| std::env::temp_dir().join("voice-icons"));

    fs::create_dir_all(&icon_dir)
        .with_context(|| format!("failed to create icon directory {}", icon_dir.display()))?;

    for (file_name, contents) in APP_INDICATOR_ICONS {
        let path = icon_dir.join(file_name);
        let should_write = fs::read_to_string(&path)
            .map(|existing| existing != *contents)
            .unwrap_or(true);

        if should_write {
            fs::write(&path, contents)
                .with_context(|| format!("failed to write icon {}", path.display()))?;
        }
    }

    Ok(icon_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_status_notifier_when_available() {
        assert_eq!(
            choose_backend_kind(true, true),
            TrayBackendKind::StatusNotifier
        );
    }

    #[test]
    fn falls_back_to_appindicator_when_sni_is_unavailable() {
        assert_eq!(
            choose_backend_kind(false, true),
            TrayBackendKind::AppIndicator
        );
    }

    #[test]
    fn uses_no_tray_when_no_backend_is_available() {
        assert_eq!(choose_backend_kind(false, false), TrayBackendKind::None);
    }

    #[test]
    fn maps_visual_states_to_icon_names() {
        assert_eq!(icon_name_for_state(VoiceActivityState::Idle), "voice-idle");
        assert_eq!(
            icon_name_for_state(VoiceActivityState::Recording),
            "voice-recording"
        );
        assert_eq!(
            icon_name_for_state(VoiceActivityState::Processing),
            "voice-processing"
        );
    }

    #[test]
    fn generated_pixmaps_use_state_color() {
        let recording = tray_icon_pixmap(VoiceActivityState::Recording);
        let processing = tray_icon_pixmap(VoiceActivityState::Processing);

        assert_eq!(recording.width, TRAY_ICON_SIZE);
        assert_eq!(recording.height, TRAY_ICON_SIZE);
        assert_ne!(recording.data, processing.data);
        assert!(
            recording
                .data
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 255, 80, 80])
        );
        assert!(
            processing
                .data
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 255, 214, 88])
        );
    }
}
