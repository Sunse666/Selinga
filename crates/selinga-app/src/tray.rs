use crate::settings::HotkeySettings;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent as GhkEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleHidden,
    TogglePause,
    Refresh,
    Reselect,
    OpenConfig,
    Quit,
}

pub struct Tray {
    _manager: GlobalHotKeyManager,
    _tray: TrayIcon,
    id_hide: tray_icon::menu::MenuId,
    id_pause: tray_icon::menu::MenuId,
    id_refresh: tray_icon::menu::MenuId,
    id_reselect: tray_icon::menu::MenuId,
    id_config: tray_icon::menu::MenuId,
    id_quit: tray_icon::menu::MenuId,
    hk_hide: Option<u32>,
    hk_pause: Option<u32>,
    hk_reselect: Option<u32>,
    item_hide: MenuItem,
    item_pause: MenuItem,
    spec_hide: String,
    spec_pause: String,
    pub failed: Vec<String>,
}

impl Tray {
    pub fn new(hotkeys: &HotkeySettings) -> Result<Self, String> {
        let item_hide =
            MenuItem::with_id("hide", format!("隐藏方框  {}", hotkeys.hide), true, None);
        let item_pause =
            MenuItem::with_id("pause", format!("暂停识别  {}", hotkeys.pause), true, None);
        let item_refresh = MenuItem::with_id("refresh", "重新识别", true, None);
        let item_reselect = MenuItem::with_id(
            "reselect",
            format!("重新框选  {}", hotkeys.reselect),
            true,
            None,
        );
        let item_config = MenuItem::with_id("config", "配置文件  settings.toml", true, None);
        let item_quit = MenuItem::with_id("quit", "退出", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &item_hide,
            &item_pause,
            &item_refresh,
            &item_reselect,
            &item_config,
            &item_quit,
        ])
        .map_err(|e| e.to_string())?;

        let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Selinga")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .build()
            .map_err(|e| e.to_string())?;

        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let mut failed = Vec::new();
        let hk_hide = register(&manager, &hotkeys.hide, &mut failed);
        let hk_pause = register(&manager, &hotkeys.pause, &mut failed);
        let hk_reselect = register(&manager, &hotkeys.reselect, &mut failed);

        Ok(Self {
            hk_hide,
            hk_pause,
            hk_reselect,
            id_hide: item_hide.id().clone(),
            id_pause: item_pause.id().clone(),
            id_refresh: item_refresh.id().clone(),
            id_reselect: item_reselect.id().clone(),
            id_config: item_config.id().clone(),
            id_quit: item_quit.id().clone(),
            item_hide,
            item_pause,
            spec_hide: hotkeys.hide.clone(),
            spec_pause: hotkeys.pause.clone(),
            failed,
            _manager: manager,
            _tray: tray,
        })
    }

    pub fn drain(&self) -> Vec<TrayCommand> {
        let mut out = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let cmd = if event.id == self.id_hide {
                TrayCommand::ToggleHidden
            } else if event.id == self.id_pause {
                TrayCommand::TogglePause
            } else if event.id == self.id_refresh {
                TrayCommand::Refresh
            } else if event.id == self.id_reselect {
                TrayCommand::Reselect
            } else if event.id == self.id_config {
                TrayCommand::OpenConfig
            } else if event.id == self.id_quit {
                TrayCommand::Quit
            } else {
                continue;
            };
            out.push(cmd);
        }
        while let Ok(event) = GhkEvent::receiver().try_recv() {
            if event.state != HotKeyState::Pressed {
                continue;
            }
            if self.hk_hide == Some(event.id) {
                out.push(TrayCommand::ToggleHidden);
            } else if self.hk_pause == Some(event.id) {
                out.push(TrayCommand::TogglePause);
            } else if self.hk_reselect == Some(event.id) {
                out.push(TrayCommand::Reselect);
            }
        }
        out
    }

    pub fn set_hidden_label(&self, hidden: bool) {
        let action = if hidden {
            "显示方框"
        } else {
            "隐藏方框"
        };
        self.item_hide
            .set_text(format!("{action}  {}", self.spec_hide));
    }

    pub fn set_paused_label(&self, paused: bool) {
        let action = if paused {
            "恢复识别"
        } else {
            "暂停识别"
        };
        self.item_pause
            .set_text(format!("{action}  {}", self.spec_pause));
    }
}

fn register(manager: &GlobalHotKeyManager, spec: &str, failed: &mut Vec<String>) -> Option<u32> {
    let Ok(hotkey) = spec.parse::<HotKey>() else {
        failed.push(spec.to_owned());
        return None;
    };
    let id = hotkey.id;
    if manager.register(hotkey).is_err() {
        failed.push(spec.to_owned());
        return None;
    }
    Some(id)
}

fn icon_rgba(size: u32) -> Vec<u8> {
    let n = size as i32;
    let half = n / 2;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let inside = (x - half + 1).abs().max((y - half + 1).abs()) <= half - 2;
            if inside {
                data.extend_from_slice(&[0x53, 0x4A, 0xB7, 0xFF]);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    data
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_specs_parse() {
        let hotkeys = HotkeySettings::default();
        for spec in [&hotkeys.hide, &hotkeys.pause, &hotkeys.reselect] {
            assert!(spec.parse::<HotKey>().is_ok(), "cannot parse {spec}");
        }
    }

    #[test]
    fn single_key_spec_is_accepted() {
        let hotkey: HotKey = "F2".parse().unwrap();
        assert_eq!(hotkey.into_string(), "F2");
    }

    #[test]
    fn garbage_spec_is_rejected() {
        assert!("Ctrl+NotAKey".parse::<HotKey>().is_err());
        assert!("".parse::<HotKey>().is_err());
    }
}
