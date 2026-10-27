use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub target_lang: String,
    pub display_mode: String,
    pub paused: bool,
    pub vlm: VlmSettings,
    pub hotkeys: HotkeySettings,
    pub autostart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySettings {
    pub hide: String,
    pub pause: String,
    pub reselect: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            hide: "Ctrl+Alt+H".to_owned(),
            pause: "Ctrl+Alt+P".to_owned(),
            reselect: "F2".to_owned(),
        }
    }
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

const KNOWN_LANGS: [&str; 4] = ["zh-CN", "en-US", "ja-JP", "ko-KR"];
const KNOWN_DISPLAY_MODES: [&str; 2] = ["translation", "bilingual"];

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_lang: KNOWN_LANGS[0].to_owned(),
            display_mode: KNOWN_DISPLAY_MODES[0].to_owned(),
            paused: false,
            vlm: VlmSettings::disabled(),
            hotkeys: HotkeySettings::default(),
            autostart: false,
        }
    }
}

impl Settings {
    pub fn load_or_recover() -> (Self, Option<PathBuf>) {
        match Self::path() {
            Some(path) => Self::recover_from(&path),
            None => (Self::default(), None),
        }
    }

    fn recover_from(path: &std::path::Path) -> (Self, Option<PathBuf>) {
        if let Some(settings) = Self::load_from(path) {
            return (settings, None);
        }
        let backup = path.with_extension("toml.broken");
        let moved = std::fs::rename(path, &backup).is_ok();
        (Self::default(), moved.then_some(backup))
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            self.save_to(&p);
        }
    }

    pub fn path() -> Option<PathBuf> {
        #[cfg(windows)]
        let base = std::env::var_os("APPDATA").map(PathBuf::from);
        #[cfg(not(windows))]
        let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
        base.map(|b| b.join("selinga").join("settings.toml"))
    }

    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim_start_matches('\u{feff}');
        let mut settings: Settings = toml::from_str(text).ok()?;
        if !KNOWN_LANGS.contains(&settings.target_lang.as_str()) {
            settings.target_lang = KNOWN_LANGS[0].to_owned();
        }
        if !KNOWN_DISPLAY_MODES.contains(&settings.display_mode.as_str()) {
            settings.display_mode = KNOWN_DISPLAY_MODES[0].to_owned();
        }
        Some(settings)
    }

    fn save_to(&self, path: &std::path::Path) {
        let Ok(text) = toml::to_string_pretty(self) else {
            return;
        };
        let Some(dir) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let tmp = path.with_extension("toml.tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
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
            hotkeys: HotkeySettings {
                hide: "Ctrl+Shift+H".to_owned(),
                pause: "Ctrl+Shift+P".to_owned(),
                reselect: "F4".to_owned(),
            },
            autostart: true,
        };
        s.save_to(&path);
        assert_eq!(Settings::load_from(&path), Some(s));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn partial_file_falls_back_per_field() {
        let path = temp_path("partial");
        std::fs::write(&path, "autostart = true\n").unwrap();
        let settings = Settings::load_from(&path).expect("partial file should parse");
        assert!(settings.autostart);
        assert_eq!(settings.target_lang, "zh-CN");
        assert_eq!(settings.display_mode, "translation");
        assert!(!settings.paused);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn utf8_bom_is_not_treated_as_corruption() {
        let path = temp_path("bom");
        let body =
            "\u{feff}autostart = true\ntarget_lang = \"zh-CN\"\ndisplay_mode = \"translation\"\n";
        std::fs::write(&path, body).unwrap();
        let settings = Settings::load_from(&path).expect("BOM file should parse");
        assert!(settings.autostart);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unreadable_file_is_moved_aside() {
        let path = temp_path("recover");
        std::fs::write(&path, "not [ valid toml {{{{").unwrap();
        let (settings, backup) = Settings::recover_from(&path);
        assert_eq!(settings, Settings::default());
        let backup = backup.expect("broken file should be backed up");
        assert!(backup.exists());
        assert!(!path.exists());
        let _ = std::fs::remove_file(&backup);
    }

    #[test]
    fn missing_file_reports_no_backup() {
        let path = temp_path("recover-missing");
        let _ = std::fs::remove_file(&path);
        let (_, backup) = Settings::recover_from(&path);
        assert!(backup.is_none());
    }

    #[test]
    fn corrupt_file_yields_none() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not [ valid toml {{{{").unwrap();
        assert_eq!(Settings::load_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_target_lang_falls_back_to_default() {
        let path = temp_path("unknown-lang");
        std::fs::write(
            &path,
            "target_lang = \"xx-XX\"\ndisplay_mode = \"bilingual\"\npaused = true\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.target_lang, "zh-CN");
        assert_eq!(s.display_mode, "bilingual");
        assert!(s.paused);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_display_mode_falls_back_to_default() {
        let path = temp_path("unknown-mode");
        std::fs::write(
            &path,
            "target_lang = \"ja-JP\"\ndisplay_mode = \"side-by-side\"\npaused = false\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.display_mode, "translation");
        assert_eq!(s.target_lang, "ja-JP");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let path = temp_path("atomic");
        let s = Settings::default();
        s.save_to(&path);
        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
        assert_eq!(Settings::load_from(&path), Some(s));
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
    fn missing_hotkey_section_uses_defaults() {
        let path = temp_path("hotkeys-missing");
        std::fs::write(
            &path,
            "target_lang = \"zh-CN\"\ndisplay_mode = \"translation\"\npaused = false\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.hotkeys.reselect, "F2");
        assert_eq!(s.hotkeys.hide, "Ctrl+Alt+H");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_matches_first_toolbar_state() {
        let d = Settings::default();
        assert_eq!(d.target_lang, "zh-CN");
        assert_eq!(d.display_mode, "translation");
        assert!(!d.paused);
    }

    #[test]
    fn serialized_file_exposes_ai_fields() {
        let text = toml::to_string_pretty(&Settings::default()).unwrap();
        for key in [
            "[vlm]",
            "enabled",
            "base_url",
            "api_key",
            "model",
            "[hotkeys]",
            "reselect",
        ] {
            assert!(text.contains(key), "missing {key} in:\n{text}");
        }
    }
}
