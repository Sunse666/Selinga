use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent as GhkEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleHidden,
    TogglePause,
    ToggleAi,
    Refresh,
    Reselect,
    Quit,
}

pub struct Tray {
    _manager: GlobalHotKeyManager,
    _tray: TrayIcon,
    id_hide: tray_icon::menu::MenuId,
    id_pause: tray_icon::menu::MenuId,
    id_ai: tray_icon::menu::MenuId,
    id_refresh: tray_icon::menu::MenuId,
    id_reselect: tray_icon::menu::MenuId,
    id_quit: tray_icon::menu::MenuId,
    hk_hide: u32,
    hk_pause: u32,
    item_hide: MenuItem,
    item_pause: MenuItem,
    item_ai: MenuItem,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let item_hide = MenuItem::with_id("hide", "隐藏方框  Ctrl+Alt+H", true, None);
        let item_pause = MenuItem::with_id("pause", "暂停识别  Ctrl+Alt+P", true, None);
        let item_ai = MenuItem::with_id("ai", "AI 识别：关", true, None);
        let item_refresh = MenuItem::with_id("refresh", "重新识别", true, None);
        let item_reselect = MenuItem::with_id("reselect", "重新框选", true, None);
        let item_quit = MenuItem::with_id("quit", "退出", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &item_hide,
            &item_pause,
            &item_ai,
            &item_refresh,
            &item_reselect,
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
        let hk_hide = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH);
        let hk_pause = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP);
        manager.register(hk_hide).map_err(|e| e.to_string())?;
        manager.register(hk_pause).map_err(|e| e.to_string())?;

        Ok(Self {
            hk_hide: hk_hide.id,
            hk_pause: hk_pause.id,
            id_hide: item_hide.id().clone(),
            id_pause: item_pause.id().clone(),
            id_ai: item_ai.id().clone(),
            id_refresh: item_refresh.id().clone(),
            id_reselect: item_reselect.id().clone(),
            id_quit: item_quit.id().clone(),
            item_hide,
            item_pause,
            item_ai,
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
            } else if event.id == self.id_ai {
                TrayCommand::ToggleAi
            } else if event.id == self.id_refresh {
                TrayCommand::Refresh
            } else if event.id == self.id_reselect {
                TrayCommand::Reselect
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
            if event.id == self.hk_hide {
                out.push(TrayCommand::ToggleHidden);
            } else if event.id == self.hk_pause {
                out.push(TrayCommand::TogglePause);
            }
        }
        out
    }

    pub fn set_hidden_label(&self, hidden: bool) {
        self.item_hide.set_text(if hidden {
            "显示方框  Ctrl+Alt+H"
        } else {
            "隐藏方框  Ctrl+Alt+H"
        });
    }

    pub fn set_paused_label(&self, paused: bool) {
        self.item_pause.set_text(if paused {
            "恢复识别  Ctrl+Alt+P"
        } else {
            "暂停识别  Ctrl+Alt+P"
        });
    }

    pub fn set_ai_label(&self, enabled: bool) {
        self.item_ai.set_text(if enabled {
            "AI 识别：开"
        } else {
            "AI 识别：关"
        });
    }
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
