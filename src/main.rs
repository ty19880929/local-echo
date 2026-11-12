#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
mod data;
mod db;
mod desktop;
mod host;
mod net;
mod peer;
mod rdp;
mod ui;

use std::borrow::Cow;

use anyhow::{Context as _, Result};
use gpui_kit::{AssetSource, QuitMode, SharedString};
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};

use crate::{core::Core, db::Db};

// 默认图标集之外、设计稿用到的 Lucide 图标，按需嵌入。
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        X,
        Circle,
        Lock,
        KeyRound,
        ShieldCheck,
        CornerLeftUp,
        Presentation,
        LogOut,
        FileArchive,
        FileImage,
        FileCode,
        UserPlus,
        Pencil,
        Trash,
        Server,
        Paperclip,
        SendHorizontal,
        Download,
        Upload,
        Monitor,
        WifiOff
    ]
);

/// 品牌图形（docs/ui/brand，去掉元数据后的精简版）。
const BRAND: [(&str, &[u8]); 2] = [
    ("brand/mark.svg", include_bytes!("../assets/brand/mark.svg")),
    (
        "brand/mark-16.svg",
        include_bytes!("../assets/brand/mark-16.svg"),
    ),
];

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = BRAND.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(
            BRAND
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

enum Startup {
    Ready(std::sync::Arc<Core>, TcpListener),
    /// 已有实例在运行，已唤起它。
    AlreadyRunning,
    Failed(String),
}

fn start(
    runtime: &tokio::runtime::Runtime,
    tx: tokio::sync::mpsc::UnboundedSender<core::Event>,
) -> Startup {
    // 单实例：端口被占用时先尝试唤起已运行的实例（设计 8.3）。
    let listener = match runtime.block_on(TcpListener::bind(("0.0.0.0", data::PORT))) {
        Ok(listener) => listener,
        Err(_) if peer::signal_show() => return Startup::AlreadyRunning,
        Err(_) => return Startup::Failed(format!("端口 {} 被其他程序占用。", data::PORT)),
    };
    let open = || -> Result<std::sync::Arc<Core>> {
        let root = data::resolve_root();
        data::ensure_layout(&root)?;
        let db = Db::open(&root.join(data::DB_FILE)).context("无法打开数据库")?;
        Core::new(root, db, runtime.handle().clone(), tx)
    };
    match open() {
        Ok(core) => Startup::Ready(core, listener),
        Err(e) => Startup::Failed(format!("{e:#}")),
    }
}

fn main() {
    let hidden = std::env::args().any(|arg| arg == "--hidden");
    let runtime = tokio::runtime::Runtime::new().expect("无法启动 tokio 运行时");
    let (tx, rx) = unbounded_channel();
    let startup = start(&runtime, tx);
    if let Startup::AlreadyRunning = startup {
        return;
    }

    gpui_kit::application()
        .with_assets(AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            match startup {
                Startup::Ready(core, listener) => {
                    ui::apply_brand_theme(cx);
                    core.runtime.spawn(peer::serve(core.clone(), listener));
                    let tray = desktop::Tray::new(core.sender())
                        .map_err(|e| eprintln!("托盘不可用：{e:#}"))
                        .ok();
                    ui::init(core, tray, rx, cx);
                    if !hidden {
                        ui::show_window(cx);
                    }
                }
                Startup::Failed(message) => ui::fatal(message, cx),
                Startup::AlreadyRunning => unreachable!(),
            }
        });
}
