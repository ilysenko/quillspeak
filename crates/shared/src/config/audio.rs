use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioInputRef {
    #[default]
    SystemDefault,
    Device {
        host: String,
        id: String,
        label: String,
    },
}

impl AudioInputRef {
    pub const fn system_default() -> Self {
        Self::SystemDefault
    }

    pub fn display_label(&self) -> &str {
        match self {
            Self::SystemDefault => "System Default",
            Self::Device { label, .. } => label,
        }
    }

    pub fn stable_key(&self) -> String {
        match self {
            Self::SystemDefault => "system_default".to_string(),
            Self::Device { host, id, .. } => format!("{host}:{id}"),
        }
    }
}
