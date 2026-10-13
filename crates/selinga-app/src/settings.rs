use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub target_lang: String,
    pub display_mode: String,
    pub paused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_lang: "zh-CN".to_owned(),
            display_mode: "translation".to_owned(),
            paused: false,
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
    fn default_matches_first_toolbar_state() {
        let d = Settings::default();
        assert_eq!(d.target_lang, "zh-CN");
        assert_eq!(d.display_mode, "translation");
        assert!(!d.paused);
    }
}
