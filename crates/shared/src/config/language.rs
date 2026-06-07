use super::INHERIT_VALUE;

pub const AUTO_LANGUAGE_VALUE: &str = "auto";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportedLanguage {
    pub code: &'static str,
    pub label: &'static str,
}

pub const SUPPORTED_LANGUAGES: &[SupportedLanguage] = &[
    lang("af", "Afrikaans"),
    lang("ar", "Arabic"),
    lang("hy", "Armenian"),
    lang("az", "Azerbaijani"),
    lang("be", "Belarusian"),
    lang("bs", "Bosnian"),
    lang("bg", "Bulgarian"),
    lang("ca", "Catalan"),
    lang("zh", "Chinese"),
    lang("hr", "Croatian"),
    lang("cs", "Czech"),
    lang("da", "Danish"),
    lang("nl", "Dutch"),
    lang("en", "English"),
    lang("et", "Estonian"),
    lang("fi", "Finnish"),
    lang("fr", "French"),
    lang("gl", "Galician"),
    lang("de", "German"),
    lang("el", "Greek"),
    lang("he", "Hebrew"),
    lang("hi", "Hindi"),
    lang("hu", "Hungarian"),
    lang("is", "Icelandic"),
    lang("id", "Indonesian"),
    lang("it", "Italian"),
    lang("ja", "Japanese"),
    lang("kn", "Kannada"),
    lang("kk", "Kazakh"),
    lang("ko", "Korean"),
    lang("lv", "Latvian"),
    lang("lt", "Lithuanian"),
    lang("mk", "Macedonian"),
    lang("ms", "Malay"),
    lang("mr", "Marathi"),
    lang("mi", "Maori"),
    lang("ne", "Nepali"),
    lang("no", "Norwegian"),
    lang("fa", "Persian"),
    lang("pl", "Polish"),
    lang("pt", "Portuguese"),
    lang("ro", "Romanian"),
    lang("ru", "Russian"),
    lang("sr", "Serbian"),
    lang("sk", "Slovak"),
    lang("sl", "Slovenian"),
    lang("es", "Spanish"),
    lang("sw", "Swahili"),
    lang("sv", "Swedish"),
    lang("tl", "Tagalog"),
    lang("ta", "Tamil"),
    lang("th", "Thai"),
    lang("tr", "Turkish"),
    lang("uk", "Ukrainian"),
    lang("ur", "Urdu"),
    lang("vi", "Vietnamese"),
    lang("cy", "Welsh"),
];

const fn lang(code: &'static str, label: &'static str) -> SupportedLanguage {
    SupportedLanguage { code, label }
}

pub fn supported_language_label(value: &str) -> Option<&'static str> {
    match value {
        INHERIT_VALUE => Some("Default"),
        AUTO_LANGUAGE_VALUE => Some("Auto Detect"),
        code => SUPPORTED_LANGUAGES
            .iter()
            .find(|language| language.code == code)
            .map(|language| language.label),
    }
}

pub(crate) fn is_supported_language_ref(input: &str, allow_inherit: bool) -> bool {
    (allow_inherit && input == INHERIT_VALUE)
        || input == AUTO_LANGUAGE_VALUE
        || SUPPORTED_LANGUAGES
            .iter()
            .any(|language| language.code == input)
}
