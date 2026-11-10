//! 托盘与开机自启（设计 8）。

use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use crate::core::Event;

pub struct Tray {
    icon: TrayIcon,
    normal: Icon,
    unread: Icon,
}

macro_rules! tray_png {
    ($theme:literal, $size:literal) => {
        (
            $theme,
            $size,
            include_bytes!(concat!(
                "../assets/tray/tray-on-",
                $theme,
                "-",
                $size,
                ".png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../assets/tray/tray-on-",
                $theme,
                "-unread-",
                $size,
                ".png"
            ))
            .as_slice(),
        )
    };
}

/// (明暗, 尺寸, 常规, 未读)。单色图形 + 品牌红未读圆点（设计 10）。
const TRAY_PNGS: [(&str, u32, &[u8], &[u8]); 8] = [
    tray_png!("light", 16),
    tray_png!("light", 20),
    tray_png!("light", 24),
    tray_png!("light", 32),
    tray_png!("dark", 16),
    tray_png!("dark", 20),
    tray_png!("dark", 24),
    tray_png!("dark", 32),
];

/// Windows：读 SystemUsesLightTheme 选 on-light / on-dark，按 AppliedDPI 选 16/20/24/32。
#[cfg(windows)]
fn tray_variant() -> (&'static str, u32) {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let light = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"))
        .is_ok_and(|v| v == 1);
    let dpi = hkcu
        .open_subkey(r"Control Panel\Desktop\WindowMetrics")
        .and_then(|k| k.get_value::<u32, _>("AppliedDPI"))
        .unwrap_or(96);
    let size = match dpi {
        0..=96 => 16,
        97..=120 => 20,
        121..=144 => 24,
        _ => 32,
    };
    (if light { "light" } else { "dark" }, size)
}

/// Linux：默认 on-dark；托盘宿主自行缩放，取 32 px 保证 2× 清晰。
#[cfg(not(windows))]
fn tray_variant() -> (&'static str, u32) {
    ("dark", 32)
}

fn decode(png: &[u8]) -> Result<Icon> {
    let image = image::load_from_memory(png)?.into_rgba8();
    let (w, h) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), w, h)?)
}

impl Tray {
    /// 创建托盘图标；菜单和点击事件转发到事件通道。
    pub fn new(tx: UnboundedSender<Event>) -> Result<Self> {
        let (theme, size) = tray_variant();
        let (_, _, normal, unread) = TRAY_PNGS
            .iter()
            .find(|(t, s, _, _)| *t == theme && *s == size)
            .expect("tray icon variant");
        let (normal, unread) = (decode(normal)?, decode(unread)?);

        let open = MenuItem::new("打开 Local Echo", true, None);
        let quit = MenuItem::new("退出", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit])?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Local Echo")
            .with_icon(normal.clone())
            .build()?;

        let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                let _ = menu_tx.send(Event::ShowWindow);
            } else if event.id == quit_id {
                let _ = menu_tx.send(Event::Quit);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                let _ = tx.send(Event::ShowWindow);
            }
            _ => {}
        }));

        Ok(Self {
            icon,
            normal,
            unread,
        })
    }

    /// 未读总数 > 0 时显示带圆点的图标（设计 8.2）。
    pub fn set_unread(&self, count: u32) {
        let (icon, tip) = match count {
            0 => (&self.normal, "Local Echo".to_string()),
            n => (&self.unread, format!("Local Echo · {n} 条未读")),
        };
        let _ = self.icon.set_icon(Some(icon.clone()));
        let _ = self.icon.set_tooltip(Some(tip));
    }
}

// ---------------------------------------------------------------- 开机自启（设计 8.4）

const AUTOSTART_NAME: &str = "Local Echo";

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(AUTOSTART_NAME))
        .is_ok()
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(AUTOSTART_NAME, &format!("\"{}\" --hidden", exe.display()))?;
    } else {
        let _ = key.delete_value(AUTOSTART_NAME);
    }
    Ok(())
}

#[cfg(not(windows))]
fn autostart_file() -> std::path::PathBuf {
    dirs::config_dir()
        .expect("no config dir")
        .join("autostart")
        .join("local-echo.desktop")
}

#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    autostart_file().is_file()
}

#[cfg(not(windows))]
pub fn set_autostart(enabled: bool) -> Result<()> {
    let file = autostart_file();
    if !enabled {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let exe = exe.display();
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(
        &file,
        format!(
            "[Desktop Entry]\nType=Application\nName={AUTOSTART_NAME}\nExec={exe} --hidden\n\
             TryExec={exe}\nIcon=local-echo\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        ),
    )?;
    Ok(())
}
