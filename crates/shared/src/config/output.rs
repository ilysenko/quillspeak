use serde::{Deserialize, Serialize};

use super::ConfigError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputAction {
    #[serde(default = "default_copy_to_clipboard")]
    pub copy_to_clipboard: bool,
    #[serde(default)]
    pub auto_paste: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptOutput {
    pub path: String,
}

impl OutputAction {
    pub fn clipboard() -> Self {
        Self {
            copy_to_clipboard: true,
            auto_paste: false,
            script: None,
        }
    }

    pub fn script(path: String) -> Self {
        Self {
            copy_to_clipboard: false,
            auto_paste: false,
            script: Some(ScriptOutput { path }),
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
        match (
            self.script.is_some(),
            self.copy_to_clipboard,
            self.auto_paste,
        ) {
            (true, true, true) => "Run script, copy, and auto paste",
            (true, true, false) => "Run script and copy to clipboard",
            (true, false, true) => "Run script and auto paste",
            (true, false, false) => "Run script",
            (false, true, true) => "Copy and auto paste",
            (false, true, false) => "Copy to clipboard",
            (false, false, true) => "Auto paste",
            (false, false, false) => "No output",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_copies_without_auto_paste_or_script() {
        let output = OutputAction::default();

        assert!(output.copy_to_clipboard);
        assert!(!output.auto_paste);
        assert_eq!(output.script, None);
    }

    #[test]
    fn script_output_only_stores_path() {
        let output = OutputAction::script("/tmp/translate".to_string());

        assert!(!output.copy_to_clipboard);
        assert!(!output.auto_paste);
        assert_eq!(
            output.script.as_ref().map(|script| script.path.as_str()),
            Some("/tmp/translate")
        );
    }
}
