use std::collections::HashSet;

use shared::{AppConfig, ModelCatalogEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRowState {
    pub entry: ModelCatalogEntry,
    pub status: ModelStatus,
    pub referenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStatus {
    Ready,
    NotInstalled,
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying { downloaded: u64, total: Option<u64> },
    Canceling { downloaded: u64, total: Option<u64> },
    Error(String),
}

impl ModelStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Ready => "Ready".to_string(),
            Self::NotInstalled => "Not installed".to_string(),
            Self::Downloading { .. } => self
                .progress_label()
                .map(|label| format!("Downloading {label}"))
                .unwrap_or_else(|| "Downloading".to_string()),
            Self::Verifying { .. } => self
                .progress_label()
                .map(|label| format!("Verifying {label}"))
                .unwrap_or_else(|| "Verifying".to_string()),
            Self::Canceling { .. } => self
                .progress_label()
                .map(|label| format!("Canceling {label}"))
                .unwrap_or_else(|| "Canceling".to_string()),
            Self::Error(error) => format!("Error: {error}"),
        }
    }

    pub fn progress_fraction(&self) -> Option<f64> {
        let (downloaded, total) = self.download_progress()?;
        let total = total?;
        if total == 0 {
            return None;
        }
        Some((downloaded as f64 / total as f64).clamp(0.0, 1.0))
    }

    pub fn progress_label(&self) -> Option<String> {
        let (downloaded, total) = self.download_progress()?;
        match total {
            Some(total) if total > 0 => {
                let percent = downloaded.saturating_mul(100) / total;
                Some(format!(
                    "{} of {} · {}%",
                    format_size(downloaded),
                    format_size(total),
                    percent
                ))
            }
            _ => Some(format!("{} downloaded", format_size(downloaded))),
        }
    }

    pub fn download_progress(&self) -> Option<(u64, Option<u64>)> {
        match self {
            Self::Downloading { downloaded, total }
            | Self::Verifying { downloaded, total }
            | Self::Canceling { downloaded, total } => Some((*downloaded, *total)),
            Self::Ready | Self::NotInstalled | Self::Error(_) => None,
        }
    }
}

pub fn referenced_models(config: &AppConfig) -> HashSet<String> {
    let mut referenced = HashSet::new();
    for shortcut in &config.shortcuts {
        referenced.insert(shortcut.model_id.clone());
    }
    referenced
}

fn format_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else {
        format!("{:.0} MiB", bytes / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloading_status_formats_known_progress() {
        let status = ModelStatus::Downloading {
            downloaded: 50 * 1024 * 1024,
            total: Some(100 * 1024 * 1024),
        };

        assert_eq!(status.progress_fraction(), Some(0.5));
        assert_eq!(
            status.progress_label().as_deref(),
            Some("50 MiB of 100 MiB · 50%")
        );
        assert_eq!(status.label(), "Downloading 50 MiB of 100 MiB · 50%");
    }

    #[test]
    fn canceling_status_keeps_progress_visible() {
        let status = ModelStatus::Canceling {
            downloaded: 2 * 1024 * 1024,
            total: None,
        };

        assert_eq!(status.progress_fraction(), None);
        assert_eq!(status.progress_label().as_deref(), Some("2 MiB downloaded"));
        assert_eq!(status.label(), "Canceling 2 MiB downloaded");
    }

    #[test]
    fn verifying_status_keeps_download_progress_visible() {
        let status = ModelStatus::Verifying {
            downloaded: 100 * 1024 * 1024,
            total: Some(100 * 1024 * 1024),
        };

        assert_eq!(status.progress_fraction(), Some(1.0));
        assert_eq!(status.label(), "Verifying 100 MiB of 100 MiB · 100%");
    }
}
