use serde::{Deserialize, Serialize};

use super::ConfigError;

pub const DEFAULT_PASTE_CUSTOM_X11: &str = "ctrl+v";
pub const DEFAULT_PASTE_CUSTOM_WAYLAND: &str = "29:1 47:1 47:0 29:0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputAction {
    #[serde(default = "default_copy_to_clipboard")]
    pub copy_to_clipboard: bool,
    #[serde(default)]
    pub paste_from_clipboard: bool,
    #[serde(default)]
    pub paste_shortcut: PasteShortcut,
    #[serde(default = "default_paste_custom_x11")]
    pub paste_custom_x11: String,
    #[serde(default = "default_paste_custom_wayland")]
    pub paste_custom_wayland: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptOutput>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteShortcut {
    #[default]
    CtrlV,
    CtrlShiftV,
    Custom,
}

impl PasteShortcut {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CtrlV => "Ctrl+V",
            Self::CtrlShiftV => "Ctrl+Shift+V",
            Self::Custom => "Custom",
        }
    }
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
            paste_from_clipboard: false,
            paste_shortcut: PasteShortcut::default(),
            paste_custom_x11: DEFAULT_PASTE_CUSTOM_X11.to_string(),
            paste_custom_wayland: DEFAULT_PASTE_CUSTOM_WAYLAND.to_string(),
            script: None,
        }
    }

    pub fn script(path: String) -> Self {
        Self {
            copy_to_clipboard: false,
            paste_from_clipboard: false,
            paste_shortcut: PasteShortcut::default(),
            paste_custom_x11: DEFAULT_PASTE_CUSTOM_X11.to_string(),
            paste_custom_wayland: DEFAULT_PASTE_CUSTOM_WAYLAND.to_string(),
            script: Some(ScriptOutput { path }),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if let Some(script) = &self.script
            && script.path.trim().is_empty()
        {
            return Err(ConfigError::EmptyScriptPath);
        }
        if self.paste_shortcut == PasteShortcut::Custom {
            if self.paste_custom_x11.trim().is_empty() {
                return Err(ConfigError::EmptyPasteShortcut("x11"));
            }
            if self.paste_custom_wayland.trim().is_empty() {
                return Err(ConfigError::EmptyPasteShortcut("wayland"));
            }
        }
        Ok(())
    }

    pub fn label(&self) -> &'static str {
        match (
            self.script.is_some(),
            self.copy_to_clipboard,
            self.paste_from_clipboard,
        ) {
            (true, true, true) => "Run script, copy, and paste",
            (true, true, false) => "Run script and copy to clipboard",
            (true, false, true) => "Run script and paste",
            (true, false, false) => "Run script",
            (false, true, true) => "Copy and paste",
            (false, true, false) => "Copy to clipboard",
            (false, false, true) => "Paste from clipboard",
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

fn default_paste_custom_x11() -> String {
    DEFAULT_PASTE_CUSTOM_X11.to_string()
}

fn default_paste_custom_wayland() -> String {
    DEFAULT_PASTE_CUSTOM_WAYLAND.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_copies_without_script() {
        let output = OutputAction::default();

        assert!(output.copy_to_clipboard);
        assert!(!output.paste_from_clipboard);
        assert_eq!(output.paste_shortcut, PasteShortcut::CtrlV);
        assert_eq!(output.paste_custom_x11, DEFAULT_PASTE_CUSTOM_X11);
        assert_eq!(output.paste_custom_wayland, DEFAULT_PASTE_CUSTOM_WAYLAND);
        assert_eq!(output.script, None);
    }

    #[test]
    fn script_output_only_stores_path() {
        let output = OutputAction::script("/tmp/translate".to_string());

        assert!(!output.copy_to_clipboard);
        assert!(!output.paste_from_clipboard);
        assert_eq!(
            output.script.as_ref().map(|script| script.path.as_str()),
            Some("/tmp/translate")
        );
    }

    #[test]
    fn custom_paste_shortcuts_round_trip() {
        let output = OutputAction {
            paste_from_clipboard: true,
            paste_shortcut: PasteShortcut::Custom,
            paste_custom_x11: "ctrl+shift+v".to_string(),
            paste_custom_wayland: "29:1 42:1 47:1 47:0 42:0 29:0".to_string(),
            ..OutputAction::default()
        };

        let encoded = toml::to_string(&output).expect("output should encode");
        let decoded = toml::from_str::<OutputAction>(&encoded)
            .expect("output should decode")
            .validate()
            .map(|_| toml::from_str::<OutputAction>(&encoded).unwrap())
            .expect("output should validate");

        assert_eq!(decoded, output);
    }
}
