use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

#[cfg(test)]
const LANGUAGE_REGISTRY_KEY: &str = "i18n_languages";
const DEFAULT_LANGUAGE: &str = "en";

#[derive(Debug, Clone, Deserialize)]
pub struct BotTexts {
    languages: Vec<LanguageInfo>,
    translations: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanguageInfo {
    pub code: String,
    pub label: String,
    #[serde(default = "default_fallback_language")]
    pub fallback: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct BotLanguageImport {
    pub language: LanguageInfo,
    pub texts: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotLanguageExport {
    pub code: String,
    pub label: String,
    pub fallback: String,
    pub bot: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawLanguageImport {
    code: String,
    label: String,
    #[serde(default = "default_fallback_language")]
    fallback: String,
    bot: HashMap<String, String>,
}

fn default_enabled() -> bool {
    true
}

fn default_fallback_language() -> String {
    DEFAULT_LANGUAGE.to_string()
}

impl Default for BotTexts {
    fn default() -> Self {
        Self::from_language_maps(default_languages(), HashMap::new())
    }
}

impl BotTexts {
    pub fn from_language_maps(
        languages: Vec<LanguageInfo>,
        translations: HashMap<String, HashMap<String, String>>,
    ) -> Self {
        let languages = normalize_languages(languages);
        let translations = translations
            .into_iter()
            .map(|(lang, entries)| (normalize_language_token(&lang), entries))
            .collect();

        Self {
            languages,
            translations,
        }
    }

    #[cfg(test)]
    pub fn from_map(map: HashMap<String, String>) -> Self {
        let languages = map
            .get(LANGUAGE_REGISTRY_KEY)
            .and_then(|raw| serde_json::from_str::<Vec<LanguageInfo>>(raw).ok())
            .filter(|langs| !langs.is_empty())
            .unwrap_or_else(default_languages);
        let languages = normalize_languages(languages);
        let mut translations: HashMap<String, HashMap<String, String>> = HashMap::new();
        let suffixes: Vec<(String, String)> = languages
            .iter()
            .map(|lang| (format!("_{}", lang.code), lang.code.clone()))
            .collect();
        let default_language = default_language_from(&languages);

        for (key, value) in map {
            if key == LANGUAGE_REGISTRY_KEY || is_non_translation_config_key(&key) {
                continue;
            }

            let mut matched = false;
            for (suffix, lang) in &suffixes {
                if let Some(base_key) = key.strip_suffix(suffix) {
                    translations
                        .entry(lang.clone())
                        .or_default()
                        .insert(base_key.to_string(), value.clone());
                    matched = true;
                    break;
                }
            }

            if !matched {
                translations
                    .entry(default_language.clone())
                    .or_default()
                    .entry(key)
                    .or_insert(value);
            }
        }

        Self {
            languages,
            translations,
        }
    }

    pub fn languages(&self) -> Vec<LanguageInfo> {
        normalize_languages(self.languages.clone())
    }

    pub fn enabled_languages(&self) -> Vec<LanguageInfo> {
        self.languages()
            .into_iter()
            .filter(|lang| lang.enabled)
            .collect()
    }

    pub fn is_supported_language(&self, lang: &str) -> bool {
        let normalized = normalize_language_token(lang);
        self.enabled_languages()
            .iter()
            .any(|candidate| candidate.code == normalized)
    }

    pub fn normalize_language(&self, language_code: Option<&str>) -> String {
        let normalized = normalize_language_token(language_code.unwrap_or(""));
        let enabled = self.enabled_languages();

        if enabled.iter().any(|lang| lang.code == normalized) {
            return normalized;
        }

        if let Some((base, _)) = normalized.split_once('-') {
            if enabled.iter().any(|lang| lang.code == base) {
                return base.to_string();
            }
        }

        self.default_language()
    }

    pub fn default_language(&self) -> String {
        default_language_from(&self.enabled_languages())
    }

    pub fn language_fallback(&self, lang: &str) -> String {
        let normalized = self.normalize_language(Some(lang));
        self.enabled_languages()
            .into_iter()
            .find(|candidate| candidate.code == normalized)
            .map(|candidate| self.normalize_language(Some(&candidate.fallback)))
            .unwrap_or_else(|| self.default_language())
    }

    pub fn language_by_code(&self, lang: &str) -> Option<LanguageInfo> {
        let normalized = normalize_language_token(lang);
        self.languages()
            .into_iter()
            .find(|candidate| candidate.code == normalized)
    }

    pub fn get_lang(&self, key: &str, lang: &str, default: &str) -> String {
        let normalized = self.normalize_language(Some(lang));
        let fallback = self.language_fallback(&normalized);
        let default_lang = self.default_language();
        let mut candidates = vec![normalized, fallback, default_lang];
        candidates.dedup();

        for candidate in candidates {
            if let Some(value) = self
                .translations
                .get(&candidate)
                .and_then(|entries| entries.get(key))
            {
                return value.clone();
            }
        }

        default.to_string()
    }

    pub fn render_lang(
        &self,
        key: &str,
        lang: &str,
        default: &str,
        vars: &[(&str, String)],
    ) -> String {
        let mut s = self.get_lang(key, lang, default);
        for (k, v) in vars {
            let placeholder = format!("{{{}}}", k);
            s = s.replace(&placeholder, v);
        }
        s
    }

    pub fn parse_language_import(format: &str, content: &str) -> Result<BotLanguageImport, String> {
        if content.len() > 256 * 1024 {
            return Err("language import is too large".to_string());
        }

        let raw: RawLanguageImport = match format.trim().to_ascii_lowercase().as_str() {
            "json" => {
                serde_json::from_str(content).map_err(|err| format!("invalid JSON: {err}"))?
            }
            "yaml" | "yml" => {
                serde_yaml::from_str(content).map_err(|err| format!("invalid YAML: {err}"))?
            }
            other => return Err(format!("unsupported import format: {other}")),
        };

        let code = normalize_language_token(&raw.code);
        if !is_valid_language_code(&code) {
            return Err(format!("invalid language code: {}", raw.code));
        }

        let fallback = normalize_language_token(&raw.fallback);
        if !is_valid_language_code(&fallback) {
            return Err(format!("invalid fallback language code: {}", raw.fallback));
        }

        if raw.label.trim().is_empty() {
            return Err("language label is required".to_string());
        }

        if raw.bot.is_empty() {
            return Err("bot translations cannot be empty".to_string());
        }

        for key in raw.bot.keys() {
            if !is_valid_translation_key(key) {
                return Err(format!("invalid translation key: {key}"));
            }
        }

        Ok(BotLanguageImport {
            language: LanguageInfo {
                code,
                label: raw.label.trim().to_string(),
                fallback,
                enabled: true,
            },
            texts: raw.bot,
        })
    }

    pub fn export_language(&self, code: &str) -> Option<BotLanguageExport> {
        let language = self.language_by_code(code)?;
        let bot = self
            .translations
            .get(&language.code)
            .cloned()
            .unwrap_or_default();

        Some(BotLanguageExport {
            code: language.code,
            label: language.label,
            fallback: language.fallback,
            bot,
        })
    }

    pub fn translation_base_keys(&self) -> Vec<String> {
        let mut keys = BTreeSet::new();
        for entries in self.translations.values() {
            keys.extend(entries.keys().cloned());
        }
        keys.into_iter().collect()
    }
}

impl BotLanguageImport {
    #[cfg(test)]
    pub fn to_config_entries(&self) -> HashMap<String, String> {
        self.texts
            .iter()
            .map(|(key, value)| (format!("{}_{}", key, self.language.code), value.clone()))
            .collect()
    }
}

pub fn default_languages() -> Vec<LanguageInfo> {
    vec![
        LanguageInfo {
            code: "vi".to_string(),
            label: "Tiếng Việt".to_string(),
            fallback: DEFAULT_LANGUAGE.to_string(),
            enabled: true,
        },
        LanguageInfo {
            code: DEFAULT_LANGUAGE.to_string(),
            label: "English".to_string(),
            fallback: DEFAULT_LANGUAGE.to_string(),
            enabled: true,
        },
    ]
}

#[cfg(test)]
pub fn default_languages_json() -> String {
    serde_json::to_string(&default_languages()).unwrap_or_else(|_| "[]".to_string())
}

fn normalize_languages(languages: Vec<LanguageInfo>) -> Vec<LanguageInfo> {
    let normalized: Vec<LanguageInfo> = languages
        .into_iter()
        .map(normalize_language_info)
        .filter(|lang| is_valid_language_code(&lang.code) && !lang.label.is_empty())
        .collect();
    if normalized.is_empty() {
        default_languages()
    } else {
        normalized
    }
}

fn normalize_language_info(mut info: LanguageInfo) -> LanguageInfo {
    info.code = normalize_language_token(&info.code);
    info.fallback = normalize_language_token(&info.fallback);
    info.label = info.label.trim().to_string();
    info
}

fn normalize_language_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn default_language_from(languages: &[LanguageInfo]) -> String {
    if languages.iter().any(|lang| lang.code == DEFAULT_LANGUAGE) {
        DEFAULT_LANGUAGE.to_string()
    } else {
        languages
            .first()
            .map(|lang| lang.code.clone())
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string())
    }
}

fn is_valid_language_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_lowercase()) {
        return false;
    }

    let rest: Vec<&str> = parts.collect();
    if rest.len() > 1 {
        return false;
    }
    if let Some(region) = rest.first() {
        if !(2..=8).contains(&region.len())
            || !region
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return false;
        }
    }

    true
}

fn is_valid_translation_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
fn is_non_translation_config_key(key: &str) -> bool {
    matches!(
        key,
        "bank_name"
            | "bank_account"
            | "bank_account_name"
            | "base_url"
            | "required_channel_enabled"
            | "required_channel_id"
            | "required_channel_url"
            | "start_viameta_enabled"
            | "netflix_enabled"
            | "netflix_price"
            | "netflix_ctv_api_key"
            | "netflix_proxy_url"
            | "netflix_get_cookie_url"
            | "netflix_regenerate_url"
            | "netflix_menu_title"
            | "netflix_menu_description"
            | "netflix_menu_note"
            | "netflix_price_label"
            | "netflix_free_label"
            | "netflix_disabled_message"
            | "netflix_session_missing_message"
            | "netflix_buy_button_text"
            | "netflix_pc_button_text"
            | "netflix_mobile_button_text"
            | "netflix_pc_guide_enabled"
            | "netflix_pc_guide_button_text"
            | "netflix_pc_guide_video_path"
            | "netflix_pc_guide_caption"
            | "netflix_pc_guide_missing_message"
            | "netflix_language_vi_guide_enabled"
            | "netflix_language_vi_guide_button_text"
            | "netflix_language_vi_guide_video_path"
            | "netflix_language_vi_guide_caption"
            | "netflix_language_vi_guide_missing_message"
            | "netflix_mobile_guide_enabled"
            | "netflix_mobile_guide_button_text"
            | "netflix_mobile_guide_video_path"
            | "netflix_mobile_guide_caption"
            | "netflix_mobile_guide_missing_message"
            | "netflix_regen_button_text"
            | "netflix_buy_again_button_text"
            | "netflix_retry_button_text"
            | "netflix_loading_message"
            | "netflix_get_error_message"
            | "netflix_regen_loading_message"
            | "netflix_regen_error_message"
            | "netflix_success_title"
            | "netflix_account_code_label"
            | "netflix_wallet_deducted_label"
            | "netflix_time_remaining_label"
            | "netflix_success_note"
            | "netflix_cookie_title"
            | "netflix_cookie_file_caption"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(entries: &[(&str, &str)]) -> BotTexts {
        BotTexts::from_map(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        )
    }

    #[test]
    fn file_backed_get_lang_reads_key_from_requested_language() {
        let texts = BotTexts::from_language_maps(
            default_languages(),
            HashMap::from([
                (
                    "vi".to_string(),
                    HashMap::from([("start".to_string(), "Xin chao".to_string())]),
                ),
                (
                    "en".to_string(),
                    HashMap::from([("start".to_string(), "Hello".to_string())]),
                ),
            ]),
        );

        assert_eq!(texts.get_lang("start", "vi", "Default"), "Xin chao");
        assert_eq!(texts.get_lang("start", "en", "Default"), "Hello");
    }

    #[test]
    fn file_backed_get_lang_falls_back_to_language_fallback() {
        let texts = BotTexts::from_language_maps(
            vec![
                LanguageInfo {
                    code: "en".to_string(),
                    label: "English".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
                LanguageInfo {
                    code: "th".to_string(),
                    label: "Thai".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
            ],
            HashMap::from([
                (
                    "en".to_string(),
                    HashMap::from([("wallet_header".to_string(), "Wallet".to_string())]),
                ),
                ("th".to_string(), HashMap::new()),
            ]),
        );

        assert_eq!(texts.get_lang("wallet_header", "th", "Default"), "Wallet");
    }

    #[test]
    fn legacy_flat_map_get_prefers_language_suffix() {
        let texts = texts(&[
            ("start", "Default"),
            ("start_vi", "Xin chao"),
            ("start_en", "Hello"),
        ]);

        assert_eq!(texts.get_lang("start", "vi", "Fallback"), "Xin chao");
        assert_eq!(texts.get_lang("start", "en", "Fallback"), "Hello");
    }

    #[test]
    fn legacy_flat_map_get_falls_back_to_base_key_then_default() {
        let texts = texts(&[("start", "Default")]);

        assert_eq!(texts.get_lang("start", "en", "Fallback"), "Default");
        assert_eq!(texts.get_lang("missing", "en", "Fallback"), "Fallback");
    }

    #[test]
    fn localized_render_uses_language_suffix_and_substitutes_vars() {
        let texts = texts(&[("join_vi", "Tham gia {channel_url}")]);

        assert_eq!(
            texts.render_lang(
                "join",
                "vi",
                "Join {channel_url}",
                &[("channel_url", "https://t.me/demo".to_string())],
            ),
            "Tham gia https://t.me/demo"
        );
    }

    #[test]
    fn language_registry_defaults_to_vietnamese_and_english() {
        let texts = BotTexts::default();
        let languages = texts.languages();

        assert_eq!(
            languages
                .iter()
                .map(|l| l.code.as_str())
                .collect::<Vec<_>>(),
            vec!["vi", "en"]
        );
        assert!(texts.is_supported_language("vi"));
        assert!(texts.is_supported_language("en"));
    }

    #[test]
    fn dynamic_language_registry_normalizes_exact_and_regional_codes() {
        let texts = texts(&[(
            "i18n_languages",
            r#"[{"code":"vi","label":"Tiếng Việt","fallback":"en","enabled":true},{"code":"en","label":"English","fallback":"en","enabled":true},{"code":"th","label":"ไทย","fallback":"en","enabled":true}]"#,
        )]);

        assert_eq!(texts.normalize_language(Some("th")), "th");
        assert_eq!(texts.normalize_language(Some("th-TH")), "th");
        assert_eq!(texts.normalize_language(Some("vi_VN")), "vi");
        assert_eq!(texts.normalize_language(Some("fr")), "en");
    }

    #[test]
    fn localized_get_uses_language_fallback_before_base_key() {
        let texts = texts(&[
            (
                "i18n_languages",
                r#"[{"code":"en","label":"English","fallback":"en","enabled":true},{"code":"th","label":"ไทย","fallback":"en","enabled":true}]"#,
            ),
            ("wallet_header", "Base wallet"),
            ("wallet_header_en", "English wallet"),
        ]);

        assert_eq!(
            texts.get_lang("wallet_header", "th", "Default"),
            "English wallet"
        );
    }

    #[test]
    fn parses_json_language_import() {
        let import = BotTexts::parse_language_import(
            "json",
            r#"{"code":"th","label":"ไทย","fallback":"en","bot":{"start":"สวัสดี","wallet_header":"ยอดเงิน {balance}"}}"#,
        )
        .unwrap();

        assert_eq!(import.language.code, "th");
        assert_eq!(import.language.label, "ไทย");
        assert_eq!(import.texts.get("start").unwrap(), "สวัสดี");
        assert_eq!(import.to_config_entries().get("start_th").unwrap(), "สวัสดี");
    }

    #[test]
    fn parses_yaml_language_import() {
        let import = BotTexts::parse_language_import(
            "yaml",
            "code: th\nlabel: ไทย\nfallback: en\nbot:\n  start: สวัสดี\n",
        )
        .unwrap();

        assert_eq!(import.language.code, "th");
        assert_eq!(import.texts.get("start").unwrap(), "สวัสดี");
    }

    #[test]
    fn rejects_invalid_language_import_code() {
        let err = BotTexts::parse_language_import(
            "json",
            r#"{"code":"../en","label":"Bad","bot":{"start":"bad"}}"#,
        )
        .unwrap_err();

        assert!(err.contains("invalid language code"));
    }
}
