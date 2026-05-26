use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::bot::texts::{BotLanguageImport, BotTexts, LanguageInfo, default_languages};

const LANGUAGE_REGISTRY_FILE: &str = "languages.json";

pub fn ensure_default_files(i18n_dir: impl AsRef<Path>) -> Result<()> {
    let i18n_dir = i18n_dir.as_ref();
    fs::create_dir_all(i18n_dir)
        .with_context(|| format!("failed to create i18n dir {}", i18n_dir.display()))?;

    let registry_path = i18n_dir.join(LANGUAGE_REGISTRY_FILE);
    if !registry_path.exists() {
        fs::write(&registry_path, pretty_json(default_languages())?)
            .with_context(|| format!("failed to write {}", registry_path.display()))?;
    }

    for language in default_languages() {
        let path = language_file_path(i18n_dir, &language.code)?;
        if !path.exists() {
            fs::write(&path, "{}\n")
                .with_context(|| format!("failed to write {}", path.display()))?;
        }
    }

    Ok(())
}

pub fn load_texts_from_dir(i18n_dir: impl AsRef<Path>) -> Result<BotTexts> {
    let i18n_dir = i18n_dir.as_ref();
    ensure_default_files(i18n_dir)?;
    let languages = load_languages(i18n_dir)?;
    let mut translations = HashMap::new();

    for language in &languages {
        let path = language_file_path(i18n_dir, &language.code)?;
        let entries = if path.exists() {
            read_translation_file(&path)?
        } else {
            HashMap::new()
        };
        translations.insert(language.code.clone(), entries);
    }

    Ok(BotTexts::from_language_maps(languages, translations))
}

pub fn save_language_import(
    i18n_dir: impl AsRef<Path>,
    import: &BotLanguageImport,
) -> Result<usize> {
    let i18n_dir = i18n_dir.as_ref();
    ensure_default_files(i18n_dir)?;

    let mut languages = load_languages(i18n_dir)?;
    upsert_language(&mut languages, import.language.clone());
    save_languages(i18n_dir, &languages)?;
    save_language_file(i18n_dir, &import.language.code, &import.texts)?;

    Ok(import.texts.len())
}

pub fn save_language_texts(
    i18n_dir: impl AsRef<Path>,
    code: &str,
    texts_for_language: &HashMap<String, String>,
) -> Result<usize> {
    let i18n_dir = i18n_dir.as_ref();
    let current = load_texts_from_dir(i18n_dir)?;
    let language = current
        .language_by_code(code)
        .ok_or_else(|| anyhow!("language not found: {code}"))?;

    save_language_file(i18n_dir, &language.code, texts_for_language)?;
    Ok(texts_for_language.len())
}

pub fn language_texts(i18n_dir: impl AsRef<Path>, code: &str) -> Result<HashMap<String, String>> {
    let texts = load_texts_from_dir(i18n_dir)?;
    let Some(export) = texts.export_language(code) else {
        return Ok(HashMap::new());
    };
    Ok(export.bot)
}

pub fn merge_i18n_dirs(source_dir: impl AsRef<Path>, target_dir: impl AsRef<Path>) -> Result<()> {
    let source_dir = source_dir.as_ref();
    let target_dir = target_dir.as_ref();
    if !source_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(target_dir)
        .with_context(|| format!("failed to create i18n dir {}", target_dir.display()))?;

    for entry in fs::read_dir(source_dir)
        .with_context(|| format!("failed to read {}", source_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source_dir.display()))?;
        let source_path = entry.path();
        let target_path = target_dir.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", source_path.display()))?;

        if file_type.is_dir() {
            if target_path.exists() {
                merge_i18n_dirs(&source_path, &target_path)?;
            } else {
                copy_dir_all(&source_path, &target_path)?;
            }
        } else if file_type.is_file() {
            merge_i18n_file(&source_path, &target_path)?;
        }
    }

    Ok(())
}

fn load_languages(i18n_dir: &Path) -> Result<Vec<LanguageInfo>> {
    let path = i18n_dir.join(LANGUAGE_REGISTRY_FILE);
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let languages = serde_json::from_str::<Vec<LanguageInfo>>(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    Ok(BotTexts::from_language_maps(languages, HashMap::new()).languages())
}

fn save_languages(i18n_dir: &Path, languages: &[LanguageInfo]) -> Result<()> {
    let path = i18n_dir.join(LANGUAGE_REGISTRY_FILE);
    fs::write(&path, pretty_json(languages)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn read_translation_file(path: &Path) -> Result<HashMap<String, String>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut entries = serde_json::from_str::<HashMap<String, String>>(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    for value in entries.values_mut() {
        *value = value.replace("\\n", "\n");
    }
    Ok(entries)
}

fn save_language_file(
    i18n_dir: &Path,
    code: &str,
    texts_for_language: &HashMap<String, String>,
) -> Result<()> {
    let path = language_file_path(i18n_dir, code)?;
    fs::write(&path, pretty_json_map(texts_for_language)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn merge_i18n_file(source_path: &Path, target_path: &Path) -> Result<()> {
    if !target_path.exists() {
        fs::copy(source_path, target_path).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source_path.display(),
                target_path.display()
            )
        })?;
        return Ok(());
    }

    if source_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return Ok(());
    }

    if source_path.file_name().and_then(|name| name.to_str()) == Some(LANGUAGE_REGISTRY_FILE) {
        let source = serde_json::from_str::<Vec<LanguageInfo>>(
            &fs::read_to_string(source_path)
                .with_context(|| format!("failed to read {}", source_path.display()))?,
        )
        .with_context(|| format!("invalid JSON in {}", source_path.display()))?;
        let target = load_languages(
            target_path
                .parent()
                .ok_or_else(|| anyhow!("invalid target path: {}", target_path.display()))?,
        )?;
        let merged = merge_language_registry(source, target);
        fs::write(target_path, pretty_json(merged)?)
            .with_context(|| format!("failed to write {}", target_path.display()))?;
        return Ok(());
    }

    let mut merged = read_translation_file(source_path)?;
    let target = read_translation_file(target_path)?;
    merged.extend(target);
    fs::write(target_path, pretty_json_map(&merged)?)
        .with_context(|| format!("failed to write {}", target_path.display()))?;
    Ok(())
}

fn merge_language_registry(
    source: Vec<LanguageInfo>,
    target: Vec<LanguageInfo>,
) -> Vec<LanguageInfo> {
    let mut merged = target;
    for language in source {
        if !merged.iter().any(|existing| existing.code == language.code) {
            merged.push(language);
        }
    }
    merged
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create dir {}", target.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source.display()))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn language_file_path(i18n_dir: &Path, code: &str) -> Result<PathBuf> {
    if !is_safe_language_code(code) {
        return Err(anyhow!("invalid language code: {code}"));
    }
    Ok(i18n_dir.join(format!("{code}.json")))
}

fn is_safe_language_code(code: &str) -> bool {
    !code.is_empty()
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !code.starts_with('-')
        && !code.ends_with('-')
}

fn upsert_language(languages: &mut Vec<LanguageInfo>, language: LanguageInfo) {
    if let Some(existing) = languages
        .iter_mut()
        .find(|candidate| candidate.code == language.code)
    {
        *existing = language;
    } else {
        languages.push(language);
    }
}

fn pretty_json<T: serde::Serialize>(value: T) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

fn pretty_json_map(map: &HashMap<String, String>) -> Result<String> {
    let sorted: BTreeMap<_, _> = map.iter().collect();
    pretty_json(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_i18n_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "botbanhang-i18n-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_texts_from_dir_reads_registry_and_language_files() {
        let dir = temp_i18n_dir("load");
        fs::write(
            dir.join(LANGUAGE_REGISTRY_FILE),
            r#"[{"code":"vi","label":"Tiếng Việt","fallback":"en","enabled":true},{"code":"en","label":"English","fallback":"en","enabled":true}]"#,
        )
        .unwrap();
        fs::write(dir.join("vi.json"), r#"{"start":"Xin chao"}"#).unwrap();
        fs::write(dir.join("en.json"), r#"{"start":"Hello"}"#).unwrap();

        let texts = load_texts_from_dir(&dir).unwrap();

        assert_eq!(texts.get_lang("start", "vi", "Default"), "Xin chao");
        assert_eq!(texts.get_lang("start", "en", "Default"), "Hello");
    }

    #[test]
    fn save_language_texts_writes_language_json_without_suffixing_keys() {
        let dir = temp_i18n_dir("save");
        ensure_default_files(&dir).unwrap();

        let count = save_language_texts(
            &dir,
            "vi",
            &HashMap::from([("start".to_string(), "Xin chao".to_string())]),
        )
        .unwrap();

        let raw = fs::read_to_string(dir.join("vi.json")).unwrap();
        assert_eq!(count, 1);
        assert!(raw.contains("\"start\""));
        assert!(!raw.contains("start_vi"));
    }

    #[test]
    fn save_language_import_upserts_registry_and_translation_file() {
        let dir = temp_i18n_dir("import");
        ensure_default_files(&dir).unwrap();
        let import = BotTexts::parse_language_import(
            "json",
            r#"{"code":"th","label":"ไทย","fallback":"en","bot":{"start":"สวัสดี"}}"#,
        )
        .unwrap();

        let imported = save_language_import(&dir, &import).unwrap();
        let texts = load_texts_from_dir(&dir).unwrap();

        assert_eq!(imported, 1);
        assert!(texts.is_supported_language("th"));
        assert_eq!(texts.get_lang("start", "th", "Default"), "สวัสดี");
        assert!(dir.join("th.json").exists());
    }

    #[test]
    fn default_languages_json_is_valid_registry_json() {
        let parsed =
            serde_json::from_str::<Vec<LanguageInfo>>(&crate::bot::texts::default_languages_json())
                .unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn checked_in_default_i18n_files_have_required_start_keys() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("i18n");
        let texts = load_texts_from_dir(&dir).unwrap();

        assert_eq!(texts.get_lang("start_btn_shop", "en", ""), "🛒 Shop");
        assert_eq!(
            texts.get_lang("start_btn_shop", "vi", ""),
            "🛒 Xem sản phẩm"
        );
        for key in [
            "shop_list_title",
            "shop_digital_warning",
            "shop_stock_auto",
            "shop_stock_manual",
            "shop_btn_wallet",
            "shop_btn_help",
            "shop_btn_notifications",
            "shop_back_btn",
        ] {
            assert!(
                !texts.get_lang(key, "en", "").is_empty(),
                "missing {key} for en"
            );
            assert!(
                !texts.get_lang(key, "vi", "").is_empty(),
                "missing {key} for vi"
            );
        }
        assert!(!texts.get_lang("language_prompt", "en", "").is_empty());
        assert!(!texts.get_lang("language_prompt", "vi", "").is_empty());
    }

    #[test]
    fn checked_in_default_i18n_files_have_crypto_payment_keys() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("i18n");
        let texts = load_texts_from_dir(&dir).unwrap();
        let required_keys = [
            "pay_binance_btn",
            "pay_bep20_btn",
            "wallet_btn_topup_usdt",
            "wallet_btn_topup_binance",
            "topup_usdt_amount_prompt",
            "topup_binance_amount_prompt",
            "topup_usdt_create_failed",
            "topup_binance_create_failed",
            "topup_usdt_instructions",
            "topup_binance_instructions",
            "topup_binance_completed",
            "topup_binance_closed",
            "topup_binance_pending",
            "topup_crypto_status_completed",
            "binance_payment_instructions",
            "binance_payment_error",
            "binance_query_error",
            "bep20_payment_instructions",
            "bep20_payment_error",
            "copy_address_btn",
            "copy_amount_btn",
            "check_crypto_btn",
            "cancel_crypto_btn",
            "crypto_action_invalid",
            "crypto_payment_not_found",
            "crypto_status_pending",
            "crypto_status_confirming",
            "crypto_status_completed",
            "crypto_status_expired",
            "crypto_status_failed",
            "crypto_status_manual_review",
            "crypto_cancel_not_pending",
            "crypto_cancelled",
            "crypto_payment_expired_message",
        ];

        for lang in ["en", "vi"] {
            for key in required_keys {
                assert!(
                    !texts.get_lang(key, lang, "").is_empty(),
                    "missing {key} for {lang}"
                );
            }
        }
        assert!(
            texts
                .get_lang("bep20_payment_instructions", "en", "")
                .contains("exactly {amount} USDT")
        );
        assert!(
            texts
                .get_lang("topup_usdt_instructions", "en", "")
                .contains("exactly {amount_usdt} USDT")
        );
        assert!(
            texts
                .get_lang("bep20_payment_instructions", "vi", "")
                .contains("đúng chính xác {amount} USDT")
        );
        assert!(
            texts
                .get_lang("topup_usdt_instructions", "vi", "")
                .contains("đúng chính xác {amount_usdt} USDT")
        );
    }

    #[test]
    fn merge_i18n_dirs_keeps_target_values_and_adds_source_keys() {
        let source = temp_i18n_dir("merge-source");
        let target = temp_i18n_dir("merge-target");
        fs::write(
            source.join(LANGUAGE_REGISTRY_FILE),
            r#"[{"code":"vi","label":"Vietnamese default","fallback":"en","enabled":true},{"code":"en","label":"English","fallback":"en","enabled":true}]"#,
        )
        .unwrap();
        fs::write(
            target.join(LANGUAGE_REGISTRY_FILE),
            r#"[{"code":"vi","label":"Tieng Viet custom","fallback":"en","enabled":true}]"#,
        )
        .unwrap();
        fs::write(
            source.join("vi.json"),
            r#"{"start":"Default start","wallet":"Wallet"}"#,
        )
        .unwrap();
        fs::write(target.join("vi.json"), r#"{"start":"Runtime start"}"#).unwrap();
        fs::write(source.join("en.json"), r#"{"start":"Start"}"#).unwrap();

        merge_i18n_dirs(&source, &target).unwrap();

        let vi = read_translation_file(&target.join("vi.json")).unwrap();
        assert_eq!(vi.get("start").map(String::as_str), Some("Runtime start"));
        assert_eq!(vi.get("wallet").map(String::as_str), Some("Wallet"));
        let en = read_translation_file(&target.join("en.json")).unwrap();
        assert_eq!(en.get("start").map(String::as_str), Some("Start"));
        let languages = load_languages(&target).unwrap();
        assert_eq!(
            languages
                .iter()
                .find(|lang| lang.code == "vi")
                .map(|lang| lang.label.as_str()),
            Some("Tieng Viet custom")
        );
        assert!(languages.iter().any(|lang| lang.code == "en"));
    }
}
