use serde::{Deserialize, Serialize};

use super::ConfigError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputAction {
    #[serde(default = "default_copy_to_clipboard")]
    pub copy_to_clipboard: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paste: Option<PasteOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptOutput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasteOutput {
    #[serde(default)]
    pub shortcut: PasteShortcut,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteShortcut {
    #[default]
    CtrlV,
    CtrlShiftV,
}

impl PasteShortcut {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::CtrlV => "ctrl_v",
            Self::CtrlShiftV => "ctrl_shift_v",
        }
    }

    pub const fn display_label(self) -> &'static str {
        match self {
            Self::CtrlV => "Ctrl+V",
            Self::CtrlShiftV => "Ctrl+Shift+V",
        }
    }

    pub fn from_wire_str(value: &str) -> Option<Self> {
        match value {
            "ctrl_v" => Some(Self::CtrlV),
            "ctrl_shift_v" => Some(Self::CtrlShiftV),
            _ => None,
        }
    }
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
            paste: None,
            script: None,
        }
    }

    pub fn script(path: String) -> Self {
        Self {
            copy_to_clipboard: false,
            paste: None,
            script: Some(ScriptOutput {
                path,
                copy_stdout_to_clipboard: false,
            }),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if self.paste.is_some() && !self.copy_to_clipboard {
            return Err(ConfigError::PasteRequiresClipboard);
        }
        if let Some(script) = &self.script
            && script.path.trim().is_empty()
        {
            return Err(ConfigError::EmptyScriptPath);
        }
        Ok(())
    }

    pub fn label(&self) -> &'static str {
        match (
            self.copy_to_clipboard,
            self.paste.is_some(),
            self.script.is_some(),
        ) {
            (true, true, true) => "Copy, paste, and run script",
            (true, true, false) => "Copy and paste",
            (true, false, true) => "Copy to clipboard and run script",
            (true, false, false) => "Copy to clipboard",
            (false, _, true) => "Run script",
            (false, _, false) => "No output",
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
    fn paste_shortcut_wire_values_round_trip() {
        assert_eq!(PasteShortcut::CtrlV.as_wire_str(), "ctrl_v");
        assert_eq!(
            PasteShortcut::from_wire_str("ctrl_v"),
            Some(PasteShortcut::CtrlV)
        );
        assert_eq!(PasteShortcut::CtrlShiftV.as_wire_str(), "ctrl_shift_v");
        assert_eq!(
            PasteShortcut::from_wire_str("ctrl_shift_v"),
            Some(PasteShortcut::CtrlShiftV)
        );
    }

    #[test]
    fn paste_shortcut_rejects_unknown_wire_values() {
        assert_eq!(PasteShortcut::from_wire_str(""), None);
        assert_eq!(PasteShortcut::from_wire_str("ctrl-v"), None);
        assert_eq!(PasteShortcut::from_wire_str("Ctrl+Shift+V"), None);
    }

    #[test]
    fn paste_shortcut_display_labels_match_settings_copy() {
        assert_eq!(PasteShortcut::CtrlV.display_label(), "Ctrl+V");
        assert_eq!(PasteShortcut::CtrlShiftV.display_label(), "Ctrl+Shift+V");
    }
}
