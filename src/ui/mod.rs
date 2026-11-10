//! 界面层：全局状态、事件分发、主窗口生命周期（设计 2.1、8）。

mod app;
mod chat;
mod dialogs;
mod kit;
mod settings;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui_kit::{
    component::{Theme, ThemeRegistry, TitleBar, WindowExt as _},
    *,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    core::{Core, Event},
    db::ContactKind,
    desktop::Tray,
};

pub use app::AppView;

/// 跨窗口生命周期保留的界面状态：窗口关闭后托盘、未读数、进度仍然有效。
pub struct AppState {
    pub core: Arc<Core>,
    pub unread: HashMap<i64, u32>,
    pub progress: HashMap<String, u64>,
    /// 失败消息的原因（只存内存），用于行内显示「发送失败」与「设备不匹配」。
    pub errors: HashMap<String, String>,
    pub auth_failed: HashSet<i64>,
    pub selected: Option<i64>,
    window: Option<AnyWindowHandle>,
    view: Option<WeakEntity<AppView>>,
    tray: Option<Tray>,
}

impl Global for AppState {}

pub fn state(cx: &App) -> &AppState {
    cx.global::<AppState>()
}

pub fn core(cx: &App) -> Arc<Core> {
    state(cx).core.clone()
}

/// 载入品牌主题（assets/theme.json：Local Echo Light / Dark），按系统明暗应用。
pub fn apply_brand_theme(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(e) = registry.load_themes_from_str(include_str!("../../assets/theme.json")) {
        eprintln!("主题载入失败：{e:#}");
        return;
    }
    let light = registry.themes().get("Local Echo Light").cloned();
    let dark = registry.themes().get("Local Echo Dark").cloned();
    if let (Some(light), Some(dark)) = (light, dark) {
        Theme::update(cx, |theme| {
            theme.apply_config(&dark);
            theme.apply_config(&light);
        });
    }
    Theme::sync_system_appearance(None, cx);
}

pub fn init(core: Arc<Core>, tray: Option<Tray>, mut rx: UnboundedReceiver<Event>, cx: &mut App) {
    cx.set_global(AppState {
        core,
        unread: HashMap::new(),
        progress: HashMap::new(),
        errors: HashMap::new(),
        auth_failed: HashSet::new(),
        selected: None,
        window: None,
        view: None,
        tray,
    });
    cx.spawn(async move |cx| {
        while let Some(event) = rx.recv().await {
            cx.update(|cx| handle_event(event, cx));
        }
    })
    .detach();
}

fn handle_event(event: Event, cx: &mut App) {
    match event {
        Event::MessageAdded(msg) => {
            let visible = {
                let state = state(cx);
                state.window.is_some() && state.selected == Some(msg.contact_id)
            };
            let from_peer = state(cx)
                .core
                .contact(msg.contact_id)
                .is_ok_and(|c| c.kind == ContactKind::Peer);
            if !msg.outgoing && from_peer && !visible {
                *cx.global_mut::<AppState>()
                    .unread
                    .entry(msg.contact_id)
                    .or_default() += 1;
                refresh_tray(cx);
            }
            update_view(cx, |view, window, cx| view.on_message(msg, window, cx));
        }
        Event::Progress { id, done } => {
            cx.global_mut::<AppState>().progress.insert(id, done);
            update_view(cx, |view, _, cx| view.refresh_chat(cx));
        }
        Event::Status { id, status, error } => {
            // 失败原因在消息行内显示（设计 03-A / 03-E / 04-B），不弹 Toast。
            let app = cx.global_mut::<AppState>();
            app.progress.remove(&id);
            match error {
                Some(error) => app.errors.insert(id.clone(), error),
                None => app.errors.remove(&id),
            };
            // 收到的文件保存完成时提示（设计 03-C「文件已保存」）。
            let saved = status == crate::db::Status::Ok
                && state(cx)
                    .core
                    .db(|db| db.message(&id))
                    .ok()
                    .flatten()
                    .is_some_and(|m| !m.outgoing && m.kind == crate::db::MsgKind::File);
            update_view(cx, |view, window, cx| {
                view.on_status(&id, status, cx);
                if saved {
                    window.push_notification(
                        gpui_kit::component::notification::Notification::success("文件已保存"),
                        cx,
                    );
                }
            });
        }
        Event::ContactsChanged => update_view(cx, |view, _, cx| view.reload_contacts(cx)),
        Event::HostAuthFailed(id) => {
            cx.global_mut::<AppState>().auth_failed.insert(id);
            update_view(cx, |view, _, cx| view.refresh_chat(cx));
        }
        Event::ShowWindow => show_window(cx),
        Event::Quit => cx.quit(),
    }
}

/// 在主窗口（若已打开）中更新主视图。
pub fn update_view(cx: &mut App, f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>)) {
    let (Some(handle), Some(view)) = (state(cx).window, state(cx).view.clone()) else {
        return;
    };
    let Some(view) = view.upgrade() else { return };
    let _ = handle.update(cx, |_, window, cx| {
        view.update(cx, |view, cx| f(view, window, cx))
    });
}

/// 已处于主窗口上下文（对话框、按钮回调）时直接更新主视图；
/// 此时不能再通过窗口句柄嵌套更新同一个窗口。
pub fn update_view_in(
    window: &mut Window,
    cx: &mut App,
    f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>),
) {
    let Some(view) = state(cx).view.as_ref().and_then(|v| v.upgrade()) else {
        return;
    };
    view.update(cx, |view, cx| f(view, window, cx));
}

pub fn refresh_tray(cx: &App) {
    let state = state(cx);
    if let Some(tray) = &state.tray {
        tray.set_unread(state.unread.values().sum());
    }
}

/// 打开主窗口；已打开则激活（设计 8.1）。
pub fn show_window(cx: &mut App) {
    if let Some(handle) = state(cx).window
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        app_id: Some("local-echo".into()),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1000.), px(680.)),
            cx,
        ))),
        window_min_size: Some(size(px(760.), px(520.))),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AppView::new(window, cx))
    }) {
        Ok((handle, view)) => {
            let state = cx.global_mut::<AppState>();
            state.window = Some(handle);
            state.view = Some(view.downgrade());
        }
        Err(e) => eprintln!("无法打开窗口：{e:#}"),
    }
}

/// 主视图释放（窗口关闭）时调用。
fn window_closed(cx: &mut App) {
    let state = cx.global_mut::<AppState>();
    state.window = None;
    state.view = None;
}

/// 启动失败时显示错误并退出（设计 11-B：「无法启动 Local Echo」，420 宽）。
pub fn fatal(message: String, cx: &mut App) {
    struct Fatal(SharedString);
    impl Render for Fatal {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            use gpui_kit::{
                assets::IconName as Ic,
                component::{
                    ActiveTheme as _, StyledExt as _,
                    button::{Button, ButtonVariants as _},
                    h_flex, v_flex,
                },
            };
            v_flex()
                .size_full()
                .bg(cx.theme().popover)
                .text_color(cx.theme().foreground)
                .child(
                    h_flex()
                        .px_6()
                        .pt(px(20.))
                        .items_start()
                        .gap_3()
                        .child(
                            div()
                                .mt(px(1.))
                                .text_color(cx.theme().danger)
                                .child(kit::icon(Ic::TriangleAlert, 20.)),
                        )
                        .child(
                            div()
                                .text_size(px(16.))
                                .line_height(px(22.))
                                .font_semibold()
                                .child("无法启动 Local Echo"),
                        ),
                )
                .child(
                    div()
                        .px_6()
                        .pt_3()
                        .pb_4()
                        .flex_1()
                        .text_size(px(14.))
                        .line_height(px(22.))
                        .text_color(cx.theme().muted_foreground)
                        .child(self.0.clone()),
                )
                .child(
                    kit::dialog_footer().child(
                        Button::new("quit")
                            .primary()
                            .label("退出")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
                )
        }
    }
    apply_brand_theme(cx);
    let options = WindowOptions {
        app_id: Some("local-echo".into()),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(420.), px(180.)),
            cx,
        ))),
        ..Default::default()
    };
    let message: SharedString = message.into();
    if gpui_kit::open_window(options, cx, move |_, cx| cx.new(|_| Fatal(message))).is_err() {
        cx.quit();
    }
}

// ---------------------------------------------------------------- 格式化

/// 消息表头时间：HH:MM（日期由分隔线给出）。
pub fn format_clock(ts: i64) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_millis_opt(ts)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

pub fn format_full_time(ts: i64) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_millis_opt(ts)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
