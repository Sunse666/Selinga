use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub target_lang: String,
    pub display_mode: String,
    pub paused: bool,
    #[serde(default = "VlmSettings::disabled")]
    pub vlm: VlmSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlmSettings {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl VlmSettings {
    fn disabled() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        }
    }

    pub fn config(&self) -> Option<selinga_translate::vlm::VlmConfig> {
        if !self.enabled || self.base_url.trim().is_empty() || self.model.trim().is_empty() {
            return None;
        }
        Some(selinga_translate::vlm::VlmConfig {
            base_url: self.base_url.trim().to_owned(),
            api_key: Some(self.api_key.trim())
                .filter(|k| !k.is_empty())
                .map(str::to_owned),
            model: self.model.trim().to_owned(),
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_lang: "zh-CN".to_owned(),
            display_mode: "translation".to_owned(),
            paused: false,
            vlm: VlmSettings::disabled(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            self.save_to(&p);
        }
    }

    fn path() -> Option<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var_os("APPDATA").map(PathBuf::from);
        #[cfg(not(windows))]
        let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
        base.map(|b| b.join("selinga").join("settings.toml"))
    }

    fn load_from(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let settings: Settings = toml::from_str(&text).ok()?;
        let known = ["zh-CN", "en-US", "ja-JP", "ko-KR"];
        if !known.contains(&settings.target_lang.as_str()) {
            return None;
        }
        if !["translation", "bilingual"].contains(&settings.display_mode.as_str()) {
            return None;
        }
        Some(settings)
    }

    fn save_to(&self, path: &std::path::Path) {
        if let Ok(text) = toml::to_string_pretty(self)
            && let Some(dir) = path.parent()
            && std::fs::create_dir_all(dir).is_ok()
        {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("selinga-test-{}-{tag}.toml", std::process::id()))
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let path = temp_path("roundtrip");
        let s = Settings {
            target_lang: "ja-JP".to_owned(),
            display_mode: "bilingual".to_owned(),
            paused: true,
            vlm: VlmSettings::disabled(),
        };
        s.save_to(&path);
        assert_eq!(Settings::load_from(&path), Some(s));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_yields_none() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not [ valid toml {{{{").unwrap();
        assert_eq!(Settings::load_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_target_lang_rejected() {
        let path = temp_path("unknown-lang");
        let s = Settings {
            target_lang: "xx-XX".to_owned(),
            display_mode: "translation".to_owned(),
            paused: false,
            vlm: VlmSettings::disabled(),
        };
        s.save_to(&path);
        assert_eq!(Settings::load_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_display_mode_rejected() {
        let path = temp_path("unknown-mode");
        let s = Settings {
            target_lang: "zh-CN".to_owned(),
            display_mode: "side-by-side".to_owned(),
            paused: false,
            vlm: VlmSettings::disabled(),
        };
        s.save_to(&path);
        assert_eq!(Settings::load_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_yields_none() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load_from(&path), None);
    }

    #[test]
    fn missing_vlm_section_defaults_to_disabled() {
        let path = temp_path("vlm-missing");
        std::fs::write(
            &path,
            "target_lang = \"zh-CN\"\ndisplay_mode = \"translation\"\npaused = false\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert!(!s.vlm.enabled);
        assert_eq!(s.vlm.config(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vlm_section_parses_into_config() {
        let path = temp_path("vlm-parse");
        std::fs::write(
            &path,
            "target_lang = \"zh-CN\"\ndisplay_mode = \"translation\"\npaused = false\n\n[vlm]\nenabled = true\nbase_url = \"http://localhost:11434/v1\"\napi_key = \"\"\nmodel = \"qwen2.5:7b\"\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        let cfg = s.vlm.config().unwrap();
        assert_eq!(cfg.base_url, "http://localhost:11434/v1");
        assert_eq!(cfg.model, "qwen2.5:7b");
        assert_eq!(cfg.api_key, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vlm_enabled_without_fields_yields_none() {
        let path = temp_path("vlm-empty");
        std::fs::write(
            &path,
            "target_lang = \"zh-CN\"\ndisplay_mode = \"translation\"\npaused = false\n\n[vlm]\nenabled = true\nbase_url = \"\"\napi_key = \"\"\nmodel = \"\"\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.vlm.config(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_matches_first_toolbar_state() {
        let d = Settings::default();
        assert_eq!(d.target_lang, "zh-CN");
        assert_eq!(d.display_mode, "translation");
        assert!(!d.paused);
    }
}
