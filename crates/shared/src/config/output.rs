use serde::{Deserialize, Serialize};

use super::ConfigError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputAction {
    #[serde(default = "default_copy_to_clipboard")]
    pub copy_to_clipboard: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptOutput {
    pub path: String,
    #[serde(default)]
    pub copy_stdout_to_clipboard: bool,
}

impl OutputAction {
    pub fn clipboard() -> Self {
        Self {
            copy_to_clipboard: true,
            script: None,
        }
    }

    pub fn script(path: String) -> Self {
        Self {
            copy_to_clipboard: false,
            script: Some(ScriptOutput {
                path,
                copy_stdout_to_clipboard: false,
            }),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if let Some(script) = &self.script
            && script.path.trim().is_empty()
        {
            return Err(ConfigError::EmptyScriptPath);
        }
        Ok(())
    }

    pub fn label(&self) -> &'static str {
        match (self.copy_to_clipboard, self.script.is_some()) {
            (true, true) => "Copy to clipboard and run script",
            (true, false) => "Copy to clipboard",
            (false, true) => "Run script",
            (false, false) => "No output",
        }
    }
}

impl Default for OutputAction {
    fn default() -> Self {
        Self::clipboard()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShortcutOutput {
    #[default]
    Default,
    Custom {
        #[serde(flatten)]
        action: OutputAction,
    },
}

impl ShortcutOutput {
    pub fn custom(action: OutputAction) -> Self {
        Self::Custom { action }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Custom { action } => action.label(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        match self {
            Self::Default => Ok(()),
            Self::Custom { action } => action.validate(),
        }
    }
}

pub enum ResolvedOutput<'a> {
    General(&'a OutputAction),
    Custom(&'a OutputAction),
}

impl ResolvedOutput<'_> {
    pub fn label(&self) -> &'static str {
        match self {
            Self::General(output) => output.label(),
            Self::Custom(output) => output.label(),
        }
    }
}

const fn default_copy_to_clipboard() -> bool {
    true
}
