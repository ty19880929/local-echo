//! 对话框（设计 4.3、5.2–5.4、6；视觉见 docs/ui 05–08）。
//! 统一结构：标题区 20 24 0 · 内容区 16 24 · 按钮区 8 24 20（右对齐，间距 8）。

use std::{collections::BTreeSet, rc::Rc};

use gpui_kit::{
    assets::IconName as Ic,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{AppState, AppView, core, format_full_time, format_size, kit, update_view_in};
use crate::{
    data,
    db::{Contact, ContactKind},
    host::{self, Entry},
    peer,
};

fn input(window: &mut Window, cx: &mut App, placeholder: &str, value: &str) -> Entity<InputState> {
    let (placeholder, value) = (placeholder.to_string(), value.to_string());
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

/// 保存后刷新列表并打开该会话。
fn open_contact(id: i64, window: &mut Window, cx: &mut App) {
    update_view_in(window, cx, |view: &mut AppView, window, cx| {
        view.reload_contacts(cx);
        view.select(id, window, cx);
    });
}

/// 打开一个由实体渲染全部内容的对话框；回车触发 `on_enter`（Dialog 把回车绑定为「确认」）。
fn open_entity_dialog<V: Render>(
    entity: Entity<V>,
    width: f32,
    on_enter: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_enter = Rc::new(on_enter);
    window.open_dialog(cx, move |dialog, _, cx| {
        let (enter, on_enter) = (entity.clone(), on_enter.clone());
        kit::dialog_shell(dialog, width, cx)
            .on_ok(move |_, window, cx| {
                enter.update(cx, |this, cx| on_enter(this, window, cx));
                false
            })
            .child(entity.clone())
    });
}

// ---------------------------------------------------------------- 联系人（设计 07）

struct ContactForm {
    editing: Option<Contact>,
    ip: Entity<InputState>,
    name: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
}

impl ContactForm {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let ip = self.ip.read(cx).value().trim().to_string();
        let name = self.name.read(cx).value().trim().to_string();
        if ip.is_empty() || name.is_empty() {
            self.error = Some("IP 和昵称都必须填写".into());
            return cx.notify();
        }
        let core = core(cx);
        if let Some(mut contact) = self.editing.clone() {
            // 编辑：直接保存，不做连接校验（设计 4.3）。
            contact.host = ip;
            contact.name = name;
            if let Err(e) = core.db(|db| db.update_contact(&contact)) {
                self.error = Some(format!("{e:#}").into());
                return cx.notify();
            }
            window.close_dialog(cx);
            update_view_in(window, cx, |view, _, cx| view.reload_contacts(cx));
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = core.runtime.spawn({
            let (core, ip) = (core.clone(), ip.clone());
            async move { peer::hello(&core, &ip).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r)
                .and_then(|peer_id| {
                    core.db(|db| match db.contact_by_peer(&peer_id)? {
                        Some(mut existing) => {
                            existing.host = ip.clone();
                            existing.name = name.clone();
                            db.update_contact(&existing)?;
                            Ok((existing.id, true))
                        }
                        None => {
                            let contact = Contact {
                                id: 0,
                                kind: ContactKind::Peer,
                                name: name.clone(),
                                host: ip.clone(),
                                port: data::PORT,
                                user: String::new(),
                                peer_id: Some(peer_id),
                                remote_dir: String::new(),
                            };
                            Ok((db.insert_contact(&contact)?, false))
                        }
                    })
                });
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((id, existed)) => {
                    window.close_dialog(cx);
                    if existed {
                        window.push_notification(
                            gpui_kit::component::notification::Notification::success(
                                "该联系人已在列表中，已更新 IP 和昵称",
                            ),
                            cx,
                        );
                    }
                    open_contact(id, window, cx);
                }
                Err(e) => {
                    this.busy = false;
                    this.error = Some(format!("{e:#}").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for ContactForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let editing = self.editing.is_some();
        let has_error = self.error.is_some();
        let ip_hint = match &self.error {
            Some(e) => kit::hint(e.clone(), true, cx),
            None if editing => kit::hint("修改 IP 后，消息将发送到新地址", false, cx),
            None => kit::hint("对方需已运行 Local Echo", false, cx),
        };
        let ip_field = kit::field(
            if editing { "IP" } else { "对方 IP" },
            v_flex()
                .gap_2()
                .child(
                    Input::new(&self.ip)
                        .disabled(self.busy)
                        .when(has_error, |this| this.border_color(theme.danger)),
                )
                .child(ip_hint),
        );
        let name_field = kit::field("昵称", Input::new(&self.name).disabled(self.busy));
        v_flex()
            .child(kit::dialog_header(
                if editing {
                    "编辑联系人"
                } else {
                    "添加联系人"
                },
                cx,
            ))
            .child(v_flex().px_6().py_4().gap_4().map(|this| {
                if editing {
                    this.child(name_field).child(ip_field)
                } else {
                    this.child(ip_field).child(name_field)
                }
            }))
            .child(
                kit::dialog_footer().child(kit::cancel_button()).child(
                    Button::new("submit")
                        .primary()
                        .min_w(px(72.))
                        .label(if editing { "保存" } else { "添加" })
                        .loading(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}

fn open_contact_form(editing: Option<Contact>, window: &mut Window, cx: &mut App) {
    let (ip, name) = editing
        .as_ref()
        .map(|c| (c.host.as_str(), c.name.as_str()))
        .unwrap_or_default();
    let ip = input(window, cx, "例如 192.168.1.23", ip);
    let name = input(window, cx, "例如 老李", name);
    let form = cx.new(|_| ContactForm {
        editing,
        ip,
        name,
        error: None,
        busy: false,
    });
    open_entity_dialog(form, 400., |f, window, cx| f.submit(window, cx), window, cx);
}

pub fn add_contact(window: &mut Window, cx: &mut App) {
    open_contact_form(None, window, cx);
}

pub fn edit_contact(contact: Contact, window: &mut Window, cx: &mut App) {
    open_contact_form(Some(contact), window, cx);
}

// ---------------------------------------------------------------- 服务器（设计 08）

#[derive(Clone)]
pub enum ServerMode {
    Add,
    Edit(Contact),
    Reauth(Contact),
}

const STEPS: [&str; 4] = ["连接服务器", "验证密码", "部署访问密钥", "验证密钥登录"];

struct ServerForm {
    mode: ServerMode,
    name: Entity<InputState>,
    addr: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    /// 授权进行到第几步；`None` 表示尚未提交。
    step: Option<usize>,
    error: Option<SharedString>,
    busy: bool,
}

impl ServerForm {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        let (host, port) = host::parse_addr(&self.addr.read(cx).value());
        let user = self.user.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        let needs_password = !matches!(self.mode, ServerMode::Edit(_));
        if name.is_empty()
            || host.is_empty()
            || user.is_empty()
            || (needs_password && password.is_empty())
        {
            self.step = None;
            self.error = Some("请填写所有字段".into());
            return cx.notify();
        }
        let core = core(cx);

        if let ServerMode::Edit(mut contact) = self.mode.clone() {
            // 编辑：直接保存，不做连接校验（设计 5.3）。
            let moved = contact.host != host || contact.port != port || contact.user != user;
            (contact.name, contact.host, contact.port, contact.user) = (name, host, port, user);
            if let Err(e) = core.db(|db| db.update_contact(&contact)) {
                self.error = Some(format!("{e:#}").into());
                return cx.notify();
            }
            if moved {
                let (forget, id) = (core.clone(), contact.id);
                core.runtime
                    .spawn(async move { host::forget(&forget, id).await });
            }
            window.close_dialog(cx);
            update_view_in(window, cx, |view, _, cx| view.reload_contacts(cx));
            return;
        }

        self.busy = true;
        self.error = None;
        self.step = Some(0);
        cx.notify();
        let (step_tx, mut step_rx) = tokio::sync::mpsc::unbounded_channel();
        let task = core.runtime.spawn({
            let (core, host, user) = (core.clone(), host.clone(), user.clone());
            async move { host::authorize(&core, &host, port, &user, &password, step_tx).await }
        });
        let mode = self.mode.clone();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(step) = step_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.step = Some(step);
                    cx.notify();
                });
            }
            let result = task
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r)
                .and_then(|home| match &mode {
                    ServerMode::Reauth(contact) => Ok(contact.id),
                    _ => {
                        let contact = Contact {
                            id: 0,
                            kind: ContactKind::Host,
                            name: name.clone(),
                            host: host.clone(),
                            port,
                            user: user.clone(),
                            peer_id: None,
                            remote_dir: home,
                        };
                        core.db(|db| db.insert_contact(&contact))
                    }
                });
            if let Ok(id) = result {
                // 重新授权后丢弃旧连接，横幅消失。
                core.runtime.spawn({
                    let core = core.clone();
                    async move { host::forget(&core, id).await }
                });
            }
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(id) => {
                    cx.global_mut::<AppState>().auth_failed.remove(&id);
                    window.close_dialog(cx);
                    open_contact(id, window, cx);
                }
                Err(e) => {
                    this.busy = false;
                    this.error = Some(format!("{e:#}").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_steps(&self, current: usize, cx: &App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let failed = self.error.clone();
        v_flex()
            .pt_3()
            .gap_1()
            .border_t_1()
            .border_color(theme.border)
            .children(STEPS.iter().enumerate().map(move |(i, label)| {
                let (icon, color, waiting) = if i < current {
                    (
                        kit::icon(Ic::CircleCheck, 16.).into_any_element(),
                        theme.success,
                        false,
                    )
                } else if i == current && failed.is_some() {
                    (
                        kit::icon(Ic::CircleX, 16.).into_any_element(),
                        theme.danger,
                        false,
                    )
                } else if i == current {
                    (
                        Spinner::new()
                            .with_size(px(16.))
                            .color(theme.primary)
                            .into_any_element(),
                        theme.primary,
                        false,
                    )
                } else {
                    (
                        kit::icon(Ic::Circle, 16.).into_any_element(),
                        theme.muted_foreground,
                        true,
                    )
                };
                v_flex()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .h(px(28.))
                            .gap_2()
                            .text_size(px(14.))
                            .text_color(if waiting {
                                theme.muted_foreground
                            } else {
                                theme.foreground
                            })
                            .child(div().text_color(color).child(icon))
                            .child(
                                div()
                                    .w_4()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child((i + 1).to_string()),
                            )
                            .child(label.to_string()),
                    )
                    .when(i == current && failed.is_some(), |this| {
                        this.child(div().pl(px(48.)).child(kit::hint(
                            failed.clone().unwrap(),
                            true,
                            cx,
                        )))
                    })
            }))
    }
}

impl Render for ServerForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (title, ok) = match self.mode {
            ServerMode::Add => ("添加服务器", "连接并添加"),
            ServerMode::Edit(_) => ("编辑服务器", "保存"),
            ServerMode::Reauth(_) => ("重新授权", "重新授权"),
        };
        let fields_opacity = if self.busy { 0.6 } else { 1. };
        let password = kit::field(
            "密码",
            Input::new(&self.password).mask_toggle().disabled(self.busy),
        );
        let body = match &self.mode {
            ServerMode::Reauth(contact) => {
                let addr = if contact.port == 22 {
                    contact.host.clone()
                } else {
                    format!("{}:{}", contact.host, contact.port)
                };
                v_flex()
                    .gap_4()
                    .child(kit::locked_field("名称", &contact.name, cx))
                    .child(kit::locked_field("地址", &addr, cx))
                    .child(kit::locked_field("用户名", &contact.user, cx))
                    .child(div().opacity(fields_opacity).child(password))
            }
            mode => v_flex()
                .gap_4()
                .opacity(fields_opacity)
                .child(kit::field(
                    "名称",
                    Input::new(&self.name).disabled(self.busy),
                ))
                .child(kit::field(
                    "地址",
                    Input::new(&self.addr).disabled(self.busy),
                ))
                .child(kit::field(
                    "用户名",
                    Input::new(&self.user).disabled(self.busy),
                ))
                .when(matches!(mode, ServerMode::Add), |this| this.child(password)),
        };
        let validation = self.step.is_none().then(|| self.error.clone()).flatten();
        v_flex()
            .child(kit::dialog_header(title, cx))
            .child(
                v_flex()
                    .px_6()
                    .py_4()
                    .gap_4()
                    .child(body)
                    .when_some(validation, |this, e| this.child(kit::hint(e, true, cx)))
                    .map(|this| match (&self.mode, self.step) {
                        (ServerMode::Edit(_), _) => this.child(kit::notice(
                            Ic::Info,
                            theme.muted_foreground,
                            "修改地址或用户名后，如无法登录，可通过「重新授权」重新部署访问密钥",
                            cx,
                        )),
                        (_, None) => this.child(kit::notice(
                            Ic::ShieldCheck,
                            theme.success,
                            "密码仅用于本次登录并部署访问密钥，不会被保存。之后上传、下载、打开终端都无需再输入密码。",
                            cx,
                        )),
                        (_, Some(current)) => this.child(self.render_steps(current, cx)),
                    }),
            )
            .child(
                kit::dialog_footer().child(kit::cancel_button()).child(
                    Button::new("submit")
                        .primary()
                        .label(ok)
                        .loading(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
    }
}

pub fn server(mode: ServerMode, window: &mut Window, cx: &mut App) {
    let contact = match &mode {
        ServerMode::Add => None,
        ServerMode::Edit(c) | ServerMode::Reauth(c) => Some(c.clone()),
    };
    let addr_value = contact
        .as_ref()
        .map(|c| {
            if c.port == 22 {
                c.host.clone()
            } else {
                format!("{}:{}", c.host, c.port)
            }
        })
        .unwrap_or_default();
    let name = input(
        window,
        cx,
        "如：测试服务器",
        contact.as_ref().map(|c| c.name.as_str()).unwrap_or(""),
    );
    let addr = input(window, cx, "192.168.1.20 或 192.168.1.20:2222", &addr_value);
    let user = input(
        window,
        cx,
        "root",
        contact.as_ref().map(|c| c.user.as_str()).unwrap_or(""),
    );
    let password = cx.new(|cx| InputState::new(window, cx).masked(true));
    let form = cx.new(|_| ServerForm {
        mode,
        name,
        addr,
        user,
        password,
        step: None,
        error: None,
        busy: false,
    });
    open_entity_dialog(form, 440., |f, window, cx| f.submit(window, cx), window, cx);
}

// ---------------------------------------------------------------- 同名文件确认（设计 05，480 宽）

pub fn same_name(
    host: bool,
    remote_dir: String,
    conflicts: Vec<(String, i64)>,
    on_continue: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let (title, body, ok, verb) = if host {
        (
            "已上传过同名文件",
            format!("继续上传将覆盖服务器上 {remote_dir} 中的同名文件。"),
            "继续上传",
            "上传",
        )
    } else {
        (
            "已发送过同名文件",
            "继续发送后，对方会再收到一份。".to_string(),
            "继续发送",
            "发送",
        )
    };
    let on_continue = Rc::new(on_continue);
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme().clone();
        let (enter, click) = (on_continue.clone(), on_continue.clone());
        kit::dialog_shell(dialog, 480., cx)
            .on_ok(move |_, window, cx| {
                enter(window, cx);
                true
            })
            .child(
                v_flex()
                    .child(kit::dialog_header(title, cx))
                    .child(
                        v_flex()
                            .px_6()
                            .py_4()
                            .gap_3()
                            .child(
                                v_flex()
                                    .id("dup-list")
                                    .max_h(px(144.))
                                    .overflow_y_scroll()
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.table)
                                    .children(conflicts.iter().map(|(name, ts)| {
                                        h_flex()
                                            .h(px(36.))
                                            .flex_none()
                                            .px_3()
                                            .gap_2()
                                            .border_b_1()
                                            .border_color(theme.table_row_border)
                                            .text_size(px(14.))
                                            .child(
                                                kit::icon(kit::file_icon(name), 16.)
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_semibold()
                                                    .child(name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .text_size(px(12.))
                                                    .text_color(theme.muted_foreground)
                                                    .child(format!(
                                                        "— 上次{verb}于 {}",
                                                        format_full_time(*ts)
                                                    )),
                                            )
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .line_height(px(22.))
                                    .child(body.clone()),
                            ),
                    )
                    .child(
                        kit::dialog_footer()
                            .pt_1()
                            .child(kit::cancel_button())
                            .child(Button::new("dup-continue").primary().label(ok).on_click(
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    click(window, cx);
                                },
                            )),
                    ),
            )
    });
}

// ---------------------------------------------------------------- 远程浏览器（设计 06，640 × 480）

#[derive(Clone, Copy, PartialEq)]
pub enum BrowseMode {
    Directory,
    Files,
}

pub enum BrowseResult {
    Directory(String),
    Files(String, Vec<Entry>),
}

type OnDone = Rc<dyn Fn(BrowseResult, &mut Window, &mut App)>;

struct RemoteBrowser {
    contact: Contact,
    mode: BrowseMode,
    dir: String,
    path: Entity<InputState>,
    entries: Vec<Entry>,
    selected: BTreeSet<String>,
    loading: bool,
    error: Option<SharedString>,
    on_done: OnDone,
    _subscription: Subscription,
}

impl RemoteBrowser {
    fn load(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
        let core = core(cx);
        let contact = self.contact.clone();
        let task = core.runtime.spawn({
            let core = core.clone();
            async move { host::list_dir(&core, &contact, &path).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok((dir, entries)) => {
                        this.path
                            .update(cx, |p, cx| p.set_value(dir.clone(), window, cx));
                        this.dir = dir;
                        this.entries = entries;
                        this.selected.clear();
                    }
                    Err(e) => this.error = Some(format!("无法读取该目录：{e:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click(&mut self, entry: &Entry, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        if entry.is_dir {
            if count >= 2 {
                self.load(host::join_remote(&self.dir, &entry.name), window, cx);
            }
        } else if self.mode == BrowseMode::Files {
            if !self.selected.remove(&entry.name) {
                self.selected.insert(entry.name.clone());
            }
            cx.notify();
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading
            || self.error.is_some()
            || (self.mode == BrowseMode::Files && self.selected.is_empty())
        {
            return;
        }
        let result = match self.mode {
            BrowseMode::Directory => BrowseResult::Directory(self.dir.clone()),
            BrowseMode::Files => BrowseResult::Files(
                self.dir.clone(),
                self.entries
                    .iter()
                    .filter(|e| self.selected.contains(&e.name))
                    .cloned()
                    .collect(),
            ),
        };
        window.close_dialog(cx);
        (self.on_done.clone())(result, window, cx);
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let state_row = |icon: AnyElement, text: SharedString, danger: bool| {
            h_flex()
                .flex_1()
                .justify_center()
                .gap_2()
                .text_size(px(14.))
                .text_color(if danger {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(icon)
                .child(text)
                .into_any_element()
        };
        if self.loading {
            return state_row(
                Spinner::new()
                    .with_size(px(16.))
                    .color(theme.primary)
                    .into_any_element(),
                "正在加载…".into(),
                false,
            );
        }
        if let Some(error) = &self.error {
            return state_row(
                kit::icon(Ic::CircleAlert, 16.)
                    .text_color(theme.danger)
                    .into_any_element(),
                error.clone(),
                true,
            );
        }
        if self.entries.is_empty() {
            return state_row(
                kit::icon(Ic::FolderOpen, 16.).into_any_element(),
                "空目录".into(),
                false,
            );
        }
        let entries = self.entries.clone();
        let selected = self.selected.clone();
        let files_mode = self.mode == BrowseMode::Files;
        uniform_list(
            "remote-entries",
            entries.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|i| {
                        let entry = entries[i].clone();
                        let is_selected = selected.contains(&entry.name);
                        let dimmed = !entry.is_dir && !files_mode;
                        let mtime = entry
                            .mtime
                            .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
                            .map(|t| {
                                t.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        h_flex()
                            .id(SharedString::from(format!("entry-{}", entry.name)))
                            .w_full()
                            .h(px(36.))
                            .px_6()
                            .gap_3()
                            .border_b_1()
                            .border_color(theme.table_row_border)
                            .text_size(px(14.))
                            .when(is_selected, |this| this.bg(theme.table_active))
                            .when(!is_selected && !dimmed, |this| {
                                this.cursor_pointer().hover(|s| s.bg(theme.table_hover))
                            })
                            .when(dimmed, |this| this.opacity(0.45))
                            .child(
                                kit::icon(
                                    if entry.is_dir {
                                        Ic::Folder
                                    } else {
                                        Ic::FileText
                                    },
                                    16.,
                                )
                                .text_color(if entry.is_dir {
                                    theme.foreground
                                } else {
                                    theme.muted_foreground
                                }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .w(px(88.))
                                    .text_right()
                                    .text_size(px(13.))
                                    .text_color(theme.muted_foreground)
                                    .child(if entry.is_dir {
                                        "—".to_string()
                                    } else {
                                        format_size(entry.size)
                                    }),
                            )
                            .child(
                                div()
                                    .w(px(128.))
                                    .text_size(px(13.))
                                    .text_color(theme.muted_foreground)
                                    .child(mtime),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.click(&entry, event.click_count(), window, cx)
                            }))
                    })
                    .collect()
            }),
        )
        .flex_1()
        .w_full()
        .into_any_element()
    }
}

impl Render for RemoteBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let files_mode = self.mode == BrowseMode::Files;
        let disabled =
            self.loading || self.error.is_some() || (files_mode && self.selected.is_empty());
        v_flex()
            .h(px(478.))
            .child(
                h_flex()
                    .h(px(56.))
                    .flex_none()
                    .px_6()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_semibold()
                            .child(if files_mode {
                                "下载文件"
                            } else {
                                "选择远程目录"
                            }),
                    )
                    .child(
                        Button::new("browser-x")
                            .ghost()
                            .xsmall()
                            .icon(kit::icon(Ic::X, 16.))
                            .text_color(theme.muted_foreground)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_none()
                    .px_6()
                    .pb_3()
                    .gap_2()
                    .child(
                        Button::new("up")
                            .secondary()
                            .icon(kit::icon(Ic::CornerLeftUp, 16.))
                            .label("上一级")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let parent = host::parent_remote(&this.dir);
                                this.load(parent, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(kit::mono(cx))
                            .text_size(px(13.))
                            .child(Input::new(&self.path)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.table)
                    .child(
                        h_flex()
                            .h(px(32.))
                            .flex_none()
                            .px_6()
                            .gap_3()
                            .bg(theme.table_head)
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(theme.table_head_foreground)
                            .child(div().w(px(16.)))
                            .child(div().flex_1().child("名称"))
                            .child(div().w(px(88.)).text_right().child("大小"))
                            .child(div().w(px(128.)).child("修改时间")),
                    )
                    .child(self.render_list(cx)),
            )
            .child(
                h_flex()
                    .h(px(64.))
                    .flex_none()
                    .px_6()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .text_color(theme.muted_foreground)
                            .when(files_mode, |this| {
                                this.child(format!("已选 {} 项", self.selected.len()))
                            }),
                    )
                    .child(kit::cancel_button())
                    .child(
                        Button::new("confirm")
                            .primary()
                            .label(if files_mode {
                                "下载所选"
                            } else {
                                "选择此目录"
                            })
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}

pub fn remote_browser(
    contact: Contact,
    mode: BrowseMode,
    on_done: impl Fn(BrowseResult, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let start = contact.remote_dir.clone();
    let path = input(window, cx, "/", &start);
    let browser = cx.new(|cx| {
        // 回车只用于路径栏跳转，不关闭对话框。
        let subscription = cx.subscribe_in(
            &path,
            window,
            |this: &mut RemoteBrowser, state, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let value = state.read(cx).value().trim().to_string();
                    this.load(value, window, cx);
                }
            },
        );
        RemoteBrowser {
            contact,
            mode,
            dir: start.clone(),
            path,
            entries: Vec::new(),
            selected: BTreeSet::new(),
            loading: true,
            error: None,
            on_done: Rc::new(on_done),
            _subscription: subscription,
        }
    });
    browser.update(cx, |b, cx| b.load(start, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        kit::dialog_shell(dialog, 640., cx)
            .h(px(480.))
            .on_ok(|_, _, _| false)
            .child(browser.clone())
    });
}
