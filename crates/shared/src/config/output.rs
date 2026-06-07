use serde::{Deserialize, Serialize};

use super::ConfigError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputAction {
    #[default]
    Clipboard,
    Script {
        path: String,
    },
}

impl OutputAction {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Clipboard => "Copy to clipboard",
            Self::Script { .. } => "Run script",
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        match self {
            Self::Clipboard => Ok(()),
            Self::Script { path } if path.trim().is_empty() => Err(ConfigError::EmptyScriptPath),
            Self::Script { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShortcutOutput {
    #[default]
    Default,
    Clipboard,
    Script {
        path: String,
    },
}

impl ShortcutOutput {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Clipboard => "Copy to clipboard",
            Self::Script { .. } => "Run script",
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        match self {
            Self::Default | Self::Clipboard => Ok(()),
            Self::Script { path } if path.trim().is_empty() => Err(ConfigError::EmptyScriptPath),
            Self::Script { .. } => Ok(()),
        }
    }
}

pub enum ResolvedOutput<'a> {
    General(&'a OutputAction),
    Clipboard,
    Script(&'a str),
}

impl ResolvedOutput<'_> {
    pub fn label(&self) -> &'static str {
        match self {
            Self::General(output) => output.label(),
            Self::Clipboard => "Copy to clipboard",
            Self::Script(_) => "Run script",
        }
    }
}
