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
    Error(String),
}

impl ModelStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Ready => "Ready".to_string(),
            Self::NotInstalled => "Not installed".to_string(),
            Self::Downloading { downloaded, total } => match total {
                Some(total) if *total > 0 => {
                    format!("Downloading {}%", downloaded.saturating_mul(100) / total)
                }
                _ => "Downloading".to_string(),
            },
            Self::Error(error) => format!("Error: {error}"),
        }
    }
}

pub fn referenced_models(config: &AppConfig) -> HashSet<String> {
    let mut referenced = HashSet::new();
    referenced.insert(config.general.default_model_id.clone());
    for shortcut in &config.shortcuts {
        let model_id = config.resolved_model_id(shortcut);
        referenced.insert(model_id.to_string());
    }
    referenced
}
