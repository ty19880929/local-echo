//! 对话框：添加 / 编辑联系人、添加 / 编辑 / 重新授权服务器、远程浏览器（设计 4.3、5.2–5.4）。

use std::{collections::BTreeSet, rc::Rc};

use gpui_kit::{
    component::{
        ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::{DialogClose, DialogFooter},
        form::{Field, Form},
        h_flex,
        input::{Input, InputEvent, InputState},
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{AppState, AppView, core, format_size, update_view_in};
use crate::{
    data,
    db::{Contact, ContactKind},
    host::{self, Entry},
    peer,
};

fn error_text(error: &Option<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().danger)
        .when_some(error.clone(), |this, e| this.child(e))
}

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

// ---------------------------------------------------------------- 联系人

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
                        window.push_notification("该联系人已在列表中，已更新 IP 和昵称", cx);
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
        let (first, second) = match self.editing {
            None => (
                Field::new().label("对方 IP").child(Input::new(&self.ip)),
                Field::new().label("昵称").child(Input::new(&self.name)),
            ),
            Some(_) => (
                Field::new().label("昵称").child(Input::new(&self.name)),
                Field::new().label("IP").child(Input::new(&self.ip)),
            ),
        };
        let hint = if self.editing.is_some() {
            "修改 IP 后，消息将发送到新地址"
        } else {
            "对方需已运行 Local Echo"
        };
        v_flex()
            .gap_3()
            .child(Form::new().child(first).child(second))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
            .child(error_text(&self.error, cx))
    }
}

fn open_contact_form(editing: Option<Contact>, window: &mut Window, cx: &mut App) {
    let (ip, name) = editing
        .as_ref()
        .map(|c| (c.host.as_str(), c.name.as_str()))
        .unwrap_or_default();
    let ip = input(window, cx, "192.168.1.23", ip);
    let name = input(window, cx, "如：老李", name);
    let title = if editing.is_some() {
        "编辑联系人"
    } else {
        "添加联系人"
    };
    let ok = if editing.is_some() {
        "保存"
    } else {
        "添加"
    };
    let form = cx.new(|_| ContactForm {
        editing,
        ip,
        name,
        error: None,
        busy: false,
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let (submit, enter) = (form.clone(), form.clone());
        dialog
            .title(title)
            .w(px(420.))
            .child(form.clone())
            // Dialog 把回车绑定为「确认」：改为提交表单，关闭由提交成功后负责。
            .on_ok(move |_, window, cx| {
                enter.update(cx, |f, cx| f.submit(window, cx));
                false
            })
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("cancel").outline().label("取消")))
                    .child(
                        Button::new("submit")
                            .primary()
                            .label(ok)
                            .loading(form.read(cx).busy)
                            .on_click(move |_, window, cx| {
                                submit.update(cx, |f, cx| f.submit(window, cx))
                            }),
                    ),
            )
    });
}

pub fn add_contact(window: &mut Window, cx: &mut App) {
    open_contact_form(None, window, cx);
}

pub fn edit_contact(contact: Contact, window: &mut Window, cx: &mut App) {
    open_contact_form(Some(contact), window, cx);
}

// ---------------------------------------------------------------- 服务器

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
                let id = contact.id;
                let core2 = core.clone();
                core.runtime
                    .spawn(async move { host::forget(&core2, id).await });
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
}

impl Render for ServerForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = matches!(self.mode, ServerMode::Reauth(_));
        let mut form = Form::new()
            .child(
                Field::new()
                    .label("名称")
                    .child(Input::new(&self.name).disabled(locked)),
            )
            .child(
                Field::new()
                    .label("地址")
                    .child(Input::new(&self.addr).disabled(locked)),
            )
            .child(
                Field::new()
                    .label("用户名")
                    .child(Input::new(&self.user).disabled(locked)),
            );
        if !matches!(self.mode, ServerMode::Edit(_)) {
            form = form.child(
                Field::new()
                    .label("密码")
                    .child(Input::new(&self.password).mask_toggle()),
            );
        }
        let note = match self.mode {
            ServerMode::Edit(_) => {
                "修改地址或用户名后，如无法登录，可通过「重新授权」重新部署访问密钥"
            }
            _ => {
                "密码仅用于本次登录并部署访问密钥，不会被保存。之后上传、下载、打开终端都无需再输入密码。"
            }
        };
        let failed = self.error.is_some();
        v_flex()
            .gap_3()
            .child(form)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(note),
            )
            .when_some(self.step, |this, current| {
                this.child(
                    v_flex()
                        .gap_1()
                        .children(STEPS.iter().enumerate().map(|(i, label)| {
                            let (mark, color) = if i < current {
                                ("✓", cx.theme().success)
                            } else if i == current && failed {
                                ("✕", cx.theme().danger)
                            } else if i == current {
                                ("…", cx.theme().foreground)
                            } else {
                                ("·", cx.theme().muted_foreground)
                            };
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(color)
                                .child(div().w_4().child(mark))
                                .child(format!("{} {label}", ["①", "②", "③", "④"][i]))
                        })),
                )
            })
            .child(error_text(&self.error, cx))
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
    let (title, ok) = match mode {
        ServerMode::Add => ("添加服务器", "连接并添加"),
        ServerMode::Edit(_) => ("编辑服务器", "保存"),
        ServerMode::Reauth(_) => ("重新授权", "重新授权"),
    };
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
    window.open_dialog(cx, move |dialog, _, cx| {
        let (submit, enter) = (form.clone(), form.clone());
        dialog
            .title(title)
            .w(px(460.))
            .child(form.clone())
            // Dialog 把回车绑定为「确认」：改为提交表单，关闭由提交成功后负责。
            .on_ok(move |_, window, cx| {
                enter.update(cx, |f, cx| f.submit(window, cx));
                false
            })
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("cancel").outline().label("取消")))
                    .child(
                        Button::new("submit")
                            .primary()
                            .label(ok)
                            .loading(form.read(cx).busy)
                            .on_click(move |_, window, cx| {
                                submit.update(cx, |f, cx| f.submit(window, cx))
                            }),
                    ),
            )
    });
}

// ---------------------------------------------------------------- 远程浏览器（设计 5.4）

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

    fn result(&self) -> BrowseResult {
        match self.mode {
            BrowseMode::Directory => BrowseResult::Directory(self.dir.clone()),
            BrowseMode::Files => BrowseResult::Files(
                self.dir.clone(),
                self.entries
                    .iter()
                    .filter(|e| self.selected.contains(&e.name))
                    .cloned()
                    .collect(),
            ),
        }
    }
}

impl Render for RemoteBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries.clone();
        let selected = self.selected.clone();
        let files_mode = self.mode == BrowseMode::Files;
        let theme = cx.theme().clone();
        let list = uniform_list(
            "remote-entries",
            entries.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|i| {
                        let entry = entries[i].clone();
                        let is_selected = selected.contains(&entry.name);
                        let dimmed = !entry.is_dir && !files_mode;
                        let icon = if entry.is_dir {
                            IconName::Folder
                        } else {
                            IconName::FileText
                        };
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
                            .px_2()
                            .py_1()
                            .gap_2()
                            .rounded(theme.radius)
                            .text_sm()
                            .cursor_pointer()
                            .when(is_selected, |this| {
                                this.bg(theme.list_active)
                                    .border_1()
                                    .border_color(theme.list_active_border)
                            })
                            .when(!is_selected, |this| this.hover(|s| s.bg(theme.list_hover)))
                            .when(dimmed, |this| this.text_color(theme.muted_foreground))
                            .child(Icon::new(icon).small())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .child(div().w_20().text_right().child(if entry.is_dir {
                                String::new()
                            } else {
                                format_size(entry.size)
                            }))
                            .child(
                                div()
                                    .w_32()
                                    .text_right()
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
        .h(px(300.));

        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("up")
                            .icon(IconName::ArrowUp)
                            .tooltip("上一级")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let parent = host::parent_remote(&this.dir);
                                this.load(parent, window, cx);
                            })),
                    )
                    .child(div().flex_1().child(Input::new(&self.path))),
            )
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .p_1()
                    .map(|this| {
                        if self.loading {
                            this.h(px(300.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Spinner::new())
                        } else if let Some(error) = &self.error {
                            this.h(px(300.))
                                .p_3()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error.clone())
                        } else if self.entries.is_empty() {
                            this.h(px(300.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(cx.theme().muted_foreground)
                                .child("空目录")
                        } else {
                            this.child(list)
                        }
                    }),
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
    let on_done: OnDone = Rc::new(on_done);
    let start = contact.remote_dir.clone();
    let path = input(window, cx, "/", &start);
    let browser = cx.new(|cx| {
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
            _subscription: subscription,
        }
    });
    browser.update(cx, |b, cx| b.load(start, window, cx));
    let title = match mode {
        BrowseMode::Directory => "选择远程目录",
        BrowseMode::Files => "下载文件",
    };
    window.open_dialog(cx, move |dialog, _, cx| {
        let b = browser.read(cx);
        let (count, files_mode, disabled) = (
            b.selected.len(),
            b.mode == BrowseMode::Files,
            b.loading || b.error.is_some(),
        );
        let ok = if files_mode {
            "下载所选"
        } else {
            "选择此目录"
        };
        let (confirm, on_done) = (browser.clone(), on_done.clone());
        dialog
            .title(title)
            .w(px(640.))
            .child(browser.clone())
            // 回车只用于路径栏跳转，不关闭对话框。
            .on_ok(|_, _, _| false)
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .when(files_mode, |this| {
                        this.child(div().flex_1().text_sm().child(format!("已选 {count} 项")))
                    })
                    .child(DialogClose::new().child(Button::new("cancel").outline().label("取消")))
                    .child(
                        Button::new("confirm")
                            .primary()
                            .label(ok)
                            .disabled(disabled || (files_mode && count == 0))
                            .on_click(move |_, window, cx| {
                                let result = confirm.read(cx).result();
                                window.close_dialog(cx);
                                on_done(result, window, cx);
                            }),
                    ),
            )
    });
}
