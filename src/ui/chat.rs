//! 会话视图：联系人与服务器共用（设计 4.4、5.4、5.5、6；视觉见 docs/ui 03 / 04）。

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use gpui_kit::{
    assets::IconName as Ic,
    base::SelectableText,
    component::{
        ActiveTheme as _, Placement, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{InputEvent, Textarea, TextareaState},
        message_scroller::{MessageScroller, MessageScrollerState},
        notification::Notification,
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{AppState, core, dialogs, format_size, kit, state};
use crate::{
    data,
    db::{Contact, ContactKind, Message, MsgKind, SameName, Status, now_ms},
    desktop, host, peer, rdp,
};

/// 同一发送方 5 分钟内的连续消息归为一组，共用一个表头（规格：组内间距 4，组间距 16）。
const GROUP_GAP_MS: i64 = 5 * 60 * 1000;

/// 侧边栏宽度（含右边框），用于由窗口宽度推算会话区宽度。
const SIDEBAR_W: f32 = 260.;
/// 会话区窄于此宽度时，服务器头部的「在终端中打开」「在远程桌面打开」收成图标按钮（v0.2 规格）。
const HEADER_COMPACT_BELOW: f32 = 720.;

pub struct ChatView {
    contact: Contact,
    messages: Vec<Message>,
    scroller: Entity<MessageScrollerState>,
    composer: Option<Entity<TextareaState>>,
    /// 图片显示尺寸缓存（按消息 id），避免每帧读取文件头。
    image_sizes: RefCell<HashMap<String, (f32, f32)>>,
    /// 「在远程桌面打开」正在探测端口 / 拉起客户端（v0.2 F1）。
    rdp_busy: bool,
    _subscriptions: Vec<Subscription>,
}

/// 图片最大 240 × 240 等比缩放，小于此尺寸按原尺寸显示（设计 03-B）。
fn fit_image(path: &std::path::Path) -> (f32, f32) {
    let (w, h) = image::image_dimensions(path)
        .map(|(w, h)| (w as f32, h as f32))
        .unwrap_or((240., 240.));
    let scale = (240. / w).min(240. / h).min(1.);
    (w * scale, h * scale)
}

/// 待发送的一项：(本地路径, 文件名, 大小)。
type Pending = (PathBuf, String, u64);

impl ChatView {
    pub fn new(contact: Contact, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let messages = core(cx)
            .db(|db| db.messages(contact.id))
            .unwrap_or_default();
        let scroller = cx.new(|cx| MessageScrollerState::new(messages.len(), cx));
        scroller.update(cx, |s, cx| s.scroll_to_end(cx));
        cx.observe(&scroller, |_, _, cx| cx.notify()).detach();

        // 窗口重新获得焦点时重新检查本地文件（v0.2 #13）：文件在外部被移动或删除后，
        // 卡片换成「文件已被移动或删除」；图片会退化为文件卡片，高度变化，需要重新测量。
        let mut subscriptions = vec![cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.scroller.update(cx, |s, cx| s.remeasure(cx));
                cx.notify();
            }
        })];
        let composer =
            (contact.kind == ContactKind::Peer).then(|| {
                let composer = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("输入消息，Enter 发送，Shift+Enter 换行")
                        .submit_on_enter(true)
                        .auto_grow(1, 6)
                });
                subscriptions.push(cx.subscribe_in(
                    &composer,
                    window,
                    |this, _, event, window, cx| match event {
                        InputEvent::PressEnter { shift: false, .. } => this.send_text(window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    },
                ));
                composer
            });
        Self {
            contact,
            messages,
            scroller,
            composer,
            image_sizes: RefCell::default(),
            rdp_busy: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn contact_id(&self) -> i64 {
        self.contact.id
    }

    pub fn set_contact(&mut self, contact: Contact, cx: &mut Context<Self>) {
        self.contact = contact;
        cx.notify();
    }

    pub fn on_message(&mut self, msg: Message, _: &mut Window, cx: &mut Context<Self>) {
        if msg.contact_id != self.contact.id {
            return;
        }
        match self.messages.iter().position(|m| m.id == msg.id) {
            Some(index) => {
                self.messages[index] = msg;
                self.scroller
                    .update(cx, |s, cx| _ = s.remeasure_items(index..index + 1, cx));
            }
            None => {
                self.messages.push(msg);
                self.scroller.update(cx, |s, cx| _ = s.append(1, cx));
            }
        }
        cx.notify();
    }

    pub fn on_status(&mut self, id: &str, status: Status, cx: &mut Context<Self>) {
        if let Some(index) = self.messages.iter().position(|m| m.id == id) {
            self.messages[index].status = status;
            // 完成 / 失败会改变卡片高度（操作行、进度条）。
            self.scroller
                .update(cx, |s, cx| _ = s.remeasure_items(index..index + 1, cx));
            cx.notify();
        }
    }

    /// 重新渲染并测量一条消息（本地文件状态变化时）。
    fn refresh_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.messages.iter().position(|m| m.id == id) {
            self.scroller
                .update(cx, |s, cx| _ = s.remeasure_items(index..index + 1, cx));
            cx.notify();
        }
    }

    fn new_message(&self, kind: MsgKind, body: String) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            contact_id: self.contact.id,
            outgoing: true,
            kind,
            body,
            path: None,
            remote_path: None,
            size: None,
            status: Status::Pending,
            ts: now_ms(),
        }
    }

    fn composer_empty(&self, cx: &App) -> bool {
        self.composer
            .as_ref()
            .is_none_or(|c| c.read(cx).value().trim().is_empty())
    }

    // ------------------------------------------------------------ 发送（联系人）

    fn send_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composer) = &self.composer else {
            return;
        };
        let text = composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        composer.update(cx, |state, cx| state.set_value("", window, cx));
        let msg = self.new_message(MsgKind::Text, text);
        self.dispatch(msg, window, cx);
    }

    /// 入库并在后台发送 / 上传 / 下载。
    fn dispatch(&mut self, msg: Message, window: &mut Window, cx: &mut Context<Self>) {
        let core = core(cx);
        cx.global_mut::<AppState>().errors.remove(&msg.id);
        if let Err(e) = core.add_message(&msg) {
            window.push_notification(Notification::error(format!("{e:#}")), cx);
            return;
        }
        let contact = self.contact.clone();
        match contact.kind {
            ContactKind::Peer => core.runtime.spawn(peer::send(core.clone(), contact, msg)),
            ContactKind::Host if msg.outgoing => {
                core.runtime.spawn(host::upload(core.clone(), contact, msg))
            }
            ContactKind::Host => core
                .runtime
                .spawn(host::download(core.clone(), contact, msg)),
        };
    }

    /// 选择、拖入或粘贴的路径：联系人会话里发送，服务器会话里上传（设计 4.4、5.5）。
    pub fn handle_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut files: Vec<Pending> = Vec::new();
        let mut skipped_dir = false;
        for path in paths {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                skipped_dir = true;
                continue;
            }
            let name = peer::file_name(&path);
            if self.contact.kind == ContactKind::Peer && data::is_image(&name) {
                self.send_image_file(path, window, cx);
            } else {
                files.push((path, name, meta.len()));
            }
        }
        if skipped_dir {
            window.push_notification(Notification::error("不支持发送文件夹"), cx);
        }
        if !files.is_empty() {
            self.confirm_same_name(files, window, cx);
        }
    }

    /// 图片复制到 files/ 并按时间戳命名后发送（设计 3.3）。
    fn send_image_file(&mut self, source: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<Message> {
            let core = core(cx);
            let name = data::image_name(&peer::file_name(&source));
            let (path, mut file) = data::create_unique(&core.files_dir(), &name)?;
            let size = std::io::copy(&mut std::fs::File::open(&source)?, &mut file)?;
            let mut msg = self.new_message(MsgKind::Image, peer::file_name(&path));
            msg.path = Some(core.store(&path));
            msg.size = Some(size);
            Ok(msg)
        })();
        match result {
            Ok(msg) => self.dispatch(msg, window, cx),
            Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
        }
    }

    /// 粘贴的图片（剪贴板内容）保存为 files/<时间戳>.<格式> 后发送。
    fn send_image_bytes(&mut self, image: &Image, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<Message> {
            use std::io::Write as _;
            let core = core(cx);
            let ext = image.format().extension();
            let name = data::image_name(&format!("x.{ext}"));
            let (path, mut file) = data::create_unique(&core.files_dir(), &name)?;
            file.write_all(image.bytes())?;
            let mut msg = self.new_message(MsgKind::Image, peer::file_name(&path));
            msg.path = Some(core.store(&path));
            msg.size = Some(image.bytes().len() as u64);
            Ok(msg)
        })();
        match result {
            Ok(msg) => self.dispatch(msg, window, cx),
            Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
        }
    }

    /// 同名文件确认（设计 6）：只看本工具的成功记录，只列出有同名记录的文件。
    fn confirm_same_name(
        &mut self,
        files: Vec<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let core = core(cx);
        let host = self.contact.kind == ContactKind::Host;
        let conflicts: Vec<(String, i64)> = files
            .iter()
            .filter_map(|(_, name, _)| {
                let remote = host::join_remote(&self.contact.remote_dir, name);
                let key = if host {
                    SameName::RemotePath(&remote)
                } else {
                    SameName::FileName(name)
                };
                let ts = core
                    .db(|db| db.last_sent(self.contact.id, key))
                    .ok()
                    .flatten()?;
                Some((name.clone(), ts))
            })
            .collect();
        if conflicts.is_empty() {
            self.send_files(files, window, cx);
            return;
        }
        let view = cx.entity().downgrade();
        dialogs::same_name(
            host,
            self.contact.remote_dir.clone(),
            conflicts,
            move |window, cx| {
                let files = files.clone();
                let _ = view.update(cx, |this, cx| this.send_files(files, window, cx));
            },
            window,
            cx,
        );
    }

    fn send_files(&mut self, files: Vec<Pending>, window: &mut Window, cx: &mut Context<Self>) {
        let core = core(cx);
        for (path, name, size) in files {
            let mut msg = self.new_message(MsgKind::File, name.clone());
            msg.path = Some(core.store(&path));
            msg.size = Some(size);
            if self.contact.kind == ContactKind::Host {
                msg.remote_path = Some(host::join_remote(&self.contact.remote_dir, &name));
            }
            self.dispatch(msg, window, cx);
        }
    }

    fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update_in(cx, |this, window, cx| this.handle_paths(paths, window, cx));
            }
        })
        .detach();
    }

    fn retry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) else {
            return;
        };
        msg.status = Status::Pending;
        let mut msg = msg.clone();
        let core = core(cx);
        if !msg.outgoing {
            // 重新下载：重建本地文件。
            let local = core.resolve(msg.path.as_deref().unwrap_or_default());
            if std::fs::File::create(&local).is_err() {
                let (path, _) = match data::create_unique(&core.files_dir(), &msg.body) {
                    Ok(created) => created,
                    Err(e) => {
                        return window.push_notification(Notification::error(format!("{e:#}")), cx);
                    }
                };
                msg.path = Some(core.store(&path));
            }
        }
        msg.ts = now_ms();
        self.dispatch(msg, window, cx);
    }

    // ------------------------------------------------------------ 服务器操作

    fn pick_remote_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        dialogs::remote_browser(
            self.contact.clone(),
            dialogs::BrowseMode::Directory,
            move |result, _, cx| {
                let dialogs::BrowseResult::Directory(dir) = result else {
                    return;
                };
                let core = core(cx);
                let _ = view.update(cx, |this, cx| {
                    this.contact.remote_dir = dir;
                    let _ = core.db(|db| db.update_contact(&this.contact));
                    cx.notify();
                });
            },
            window,
            cx,
        );
    }

    fn pick_downloads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        dialogs::remote_browser(
            self.contact.clone(),
            dialogs::BrowseMode::Files,
            move |result, window, cx| {
                let dialogs::BrowseResult::Files(dir, entries) = result else {
                    return;
                };
                let _ = view.update(cx, |this, cx| {
                    let core = core(cx);
                    for entry in entries {
                        let name = data::sanitize_file_name(&entry.name);
                        let Ok((path, _)) = data::create_unique(&core.files_dir(), &name) else {
                            continue;
                        };
                        let mut msg = this.new_message(MsgKind::File, entry.name.clone());
                        msg.outgoing = false;
                        msg.path = Some(core.store(&path));
                        msg.remote_path = Some(host::join_remote(&dir, &entry.name));
                        msg.size = Some(entry.size);
                        this.dispatch(msg, window, cx);
                    }
                });
            },
            window,
            cx,
        );
    }

    fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = host::open_terminal(&core(cx), &self.contact) {
            if e.is::<host::AuthFailed>() {
                cx.global_mut::<AppState>()
                    .auth_failed
                    .insert(self.contact.id);
            }
            window.push_notification(Notification::error(format!("{e:#}")), cx);
        }
    }

    /// 在远程桌面打开（v0.2 F1，设计 5.7）：探测 RDP 端口（最多 3 秒）→ 拉起本机客户端。
    /// 成功时不提示（客户端窗口本身就是反馈）；端口不通、拉起失败弹错误 Toast；
    /// Linux 找不到客户端时弹出安装提示对话框。
    fn open_rdp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rdp_busy {
            return;
        }
        self.rdp_busy = true;
        cx.notify();
        let contact = self.contact.clone();
        let task = core(cx).runtime.spawn(async move {
            rdp::probe(&contact.host, contact.rdp_port).await?;
            tokio::task::spawn_blocking(move || rdp::launch(&contact))
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r)
                .map_err(|e| {
                    if e.is::<rdp::NoClient>() {
                        e
                    } else {
                        anyhow::anyhow!("无法启动远程桌面客户端：{e:#}")
                    }
                })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.rdp_busy = false;
                cx.notify();
                match result {
                    Ok(()) => {}
                    Err(e) if e.is::<rdp::NoClient>() => dialogs::rdp_missing(window, cx),
                    Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------ 渲染

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let c = &self.contact;
        // 会话区 < 720（窗口 < 980）时，后两个按钮收成 32 × 32 图标按钮 + Tooltip（v0.2 规格）。
        let compact = window.viewport_size().width.as_f32() - SIDEBAR_W < HEADER_COMPACT_BELOW;
        match c.kind {
            ContactKind::Peer => h_flex()
                .h(px(56.))
                .flex_none()
                .px_6()
                .gap_3()
                .border_b_1()
                .border_color(theme.border)
                .child(kit::avatar_text(&c.name, cx))
                .child(
                    v_flex()
                        .child(
                            div()
                                .text_size(px(16.))
                                .line_height(px(22.))
                                .font_semibold()
                                .child(c.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(c.host.clone()),
                        ),
                ),
            ContactKind::Host => h_flex()
                .h(px(76.))
                .flex_none()
                .px_6()
                .gap_3()
                .border_b_1()
                .border_color(theme.border)
                .child(kit::avatar_server(cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(16.))
                                .line_height(px(22.))
                                .font_semibold()
                                .truncate()
                                .child(c.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(theme.muted_foreground)
                                .child(format!("{}@{}:{}", c.user, c.host, c.port)),
                        )
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_2()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(theme.muted_foreground)
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .child(format!("远程目录：{}", c.remote_dir)),
                                )
                                .child(
                                    kit::link_text("更改", cx)
                                        .id("change-dir")
                                        .flex_none()
                                        .underline()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_remote_dir(window, cx)
                                        })),
                                ),
                        ),
                )
                .child(
                    Button::new("download")
                        .secondary()
                        .icon(kit::icon(Ic::Download, 16.))
                        .label("下载文件")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.pick_downloads(window, cx)),
                        ),
                )
                .child(
                    Button::new("terminal")
                        .secondary()
                        .icon(kit::icon(Ic::SquareTerminal, 16.))
                        .map(|b| {
                            if compact {
                                b.tooltip("在终端中打开")
                                    .tooltip_placement(Placement::Bottom)
                            } else {
                                b.label("在终端中打开")
                            }
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_terminal(window, cx)),
                        ),
                )
                .child(self.render_rdp_button(compact, cx)),
        }
    }

    /// 「在远程桌面打开」：常规 / 悬停 / 检测中（Spinner +「正在连接…」，整体 70% 不透明，
    /// 不可点击，按钮保持原宽）。图标按钮只显示 Spinner，Tooltip 改为「正在连接…」。
    fn render_rdp_button(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let busy = self.rdp_busy;
        let tooltip = if busy {
            "正在连接…".to_string()
        } else {
            format!(
                "用远程桌面连接 {}:{}",
                self.contact.host, self.contact.rdp_port
            )
        };
        let button = Button::new("rdp")
            .secondary()
            .icon(kit::icon(Ic::Monitor, 16.))
            .loading(busy)
            .tooltip(tooltip)
            .tooltip_placement(Placement::Bottom)
            .when(!compact, |b| {
                b.min_w(px(146.)).label(if busy {
                    "正在连接…"
                } else {
                    "在远程桌面打开"
                })
            })
            .on_click(cx.listener(|this, _, window, cx| this.open_rdp(window, cx)));
        // 按钮加载态自带 80% 不透明，外层再乘 0.875 得到规格的 70%。
        div()
            .flex_none()
            .when(busy, |d| d.opacity(0.875))
            .child(button)
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match &self.composer {
            Some(composer) => {
                let paste_view = cx.entity().downgrade();
                let empty = self.composer_empty(cx);
                // 单行时高 60（v0.2 U3）= 上下内边距 12 + 36，输入多行时向上增高，此时不要求对齐。
                // 内边距写成像素：rem 是 14 px，py_3 只有 10.5，底对齐时内容会偏下。
                h_flex()
                    .flex_none()
                    .min_h(kit::footer_height())
                    .px_4()
                    .py(px(12.))
                    .gap_2()
                    .items_end()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("attach")
                            .ghost()
                            .size(px(36.))
                            .icon(kit::icon(Ic::Paperclip, 18.))
                            .tooltip("发送文件或图片")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_files(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Textarea::new(composer).on_paste(move |item, window, cx| {
                                let view = paste_view.clone();
                                for entry in item.entries() {
                                    match entry {
                                        ClipboardEntry::Image(image) => {
                                            let image = image.clone();
                                            let _ = view.update(cx, |this, cx| {
                                                this.send_image_bytes(&image, window, cx)
                                            });
                                            return true;
                                        }
                                        ClipboardEntry::ExternalPaths(paths) => {
                                            let paths = paths.paths().to_vec();
                                            let _ = view.update(cx, |this, cx| {
                                                this.handle_paths(paths, window, cx)
                                            });
                                            return true;
                                        }
                                        ClipboardEntry::String(_) => {}
                                    }
                                }
                                false
                            })),
                    )
                    .child(
                        Button::new("send")
                            .primary()
                            .size(px(36.))
                            .icon(kit::icon(Ic::SendHorizontal, 18.))
                            .tooltip("发送")
                            .when(empty, |b| b.opacity(0.45))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_text(window, cx)),
                            ),
                    )
                    .into_any_element()
            }
            // 服务器底栏高 60（v0.2 U3）：拖拽区与「上传文件」从 40 改为 36。
            None => h_flex()
                .flex_none()
                .h(kit::footer_height())
                .px_4()
                .py(px(12.))
                .gap_3()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h(px(36.))
                        .px_3()
                        .gap_2()
                        .border_1()
                        .border_dashed()
                        .border_color(theme.input)
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .child(kit::icon(Ic::Upload, 16.))
                        .child(div().truncate().child(format!(
                            "拖拽文件到此处，上传到 {}",
                            self.contact.remote_dir
                        ))),
                )
                .child(
                    Button::new("upload")
                        .primary()
                        .h(px(36.))
                        .px_4()
                        .label("上传文件")
                        .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx))),
                )
                .into_any_element(),
        }
    }

    /// 拖入文件覆盖层（设计 03-F / 04-C）。
    fn render_drop_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let text = match self.contact.kind {
            ContactKind::Peer => format!("松开以发送给 {}", self.contact.name),
            ContactKind::Host => format!("松开以上传到 {}", self.contact.remote_dir),
        };
        div()
            .id("drop-overlay")
            .absolute()
            .inset_0()
            .p_3()
            .flex()
            .bg(theme.drop_target)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_dashed()
                    .border_color(theme.drag_border)
                    .child(
                        v_flex()
                            .gap_3()
                            .child(kit::icon(Ic::Upload, 32.).text_color(theme.primary))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .line_height(px(28.))
                                    .font_semibold()
                                    .child(text),
                            ),
                    ),
            )
    }
}

// ---------------------------------------------------------------- 消息行

/// 渲染第 `index` 条消息。只读取数据，不持有视图的可变借用。
fn render_row(this: &ChatView, index: usize, view: &WeakEntity<ChatView>, cx: &App) -> AnyElement {
    let theme = cx.theme().clone();
    let msg = &this.messages[index];
    let contact = &this.contact;
    let prev = index.checked_sub(1).map(|i| &this.messages[i]);
    let new_day = prev.is_none_or(|p| !kit::same_day(p.ts, msg.ts));
    let group_start =
        new_day || prev.is_none_or(|p| p.outgoing != msg.outgoing || msg.ts - p.ts > GROUP_GAP_MS);
    let time = super::format_clock(msg.ts);
    let header = match (contact.kind, msg.outgoing) {
        (ContactKind::Peer, false) => format!("{} · {time}", contact.name),
        (ContactKind::Peer, true) => time,
        (ContactKind::Host, true) => format!("上传 · {time}"),
        (ContactKind::Host, false) => format!("下载 · {time}"),
    };
    let header = div()
        .text_size(px(12.))
        .text_color(theme.muted_foreground)
        .child(header);
    let body = render_body(this, msg, view, cx);

    let content = if msg.outgoing {
        v_flex()
            .w_full()
            .items_end()
            .gap_1()
            .when(group_start, |this| this.child(header))
            .child(body)
    } else {
        let avatar = if !group_start {
            div().w(px(32.)).flex_none()
        } else {
            match contact.kind {
                ContactKind::Peer => kit::avatar_text(&contact.name, cx),
                ContactKind::Host => kit::avatar_server(cx),
            }
        };
        h_flex().w_full().items_start().gap_2().child(avatar).child(
            v_flex()
                .items_start()
                .gap_1()
                .when(group_start, |this| this.child(header))
                .child(body),
        )
    };
    v_flex()
        .w_full()
        .px_6()
        .pt(px(if group_start { 16. } else { 4. }))
        .gap_4()
        .when(new_day, |this| {
            this.child(kit::date_separator(kit::date_label(msg.ts), cx))
        })
        .child(content)
        .into_any_element()
}

fn render_body(
    this: &ChatView,
    msg: &Message,
    view: &WeakEntity<ChatView>,
    cx: &App,
) -> AnyElement {
    let contact = &this.contact;
    let core = &state(cx).core;
    let local = msg.path.as_deref().map(|p| core.resolve(p));
    match msg.kind {
        MsgKind::Text => render_text(msg, contact, view, cx),
        MsgKind::Image
            if msg.status == Status::Ok && local.as_ref().is_some_and(|p| p.exists()) =>
        {
            let path = local.unwrap();
            let open = path.clone();
            let (view, id) = (view.clone(), msg.id.clone());
            let (w, h) = *this
                .image_sizes
                .borrow_mut()
                .entry(msg.id.clone())
                .or_insert_with(|| fit_image(&path));
            div()
                .id(SharedString::from(format!("img-{}", msg.id)))
                .cursor_pointer()
                .border_1()
                .border_color(cx.theme().border)
                .on_click(move |_, window, cx| open_local(&view, &id, &open, false, window, cx))
                .child(img(path).w(px(w)).h(px(h)).object_fit(ObjectFit::Contain))
                .into_any_element()
        }
        _ => render_file(msg, contact, local, view, cx).into_any_element(),
    }
}

/// 文本气泡与发送状态（设计 03-A / 03-E）。
fn render_text(
    msg: &Message,
    contact: &Contact,
    view: &WeakEntity<ChatView>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme().clone();
    let (bg, fg) = if msg.outgoing {
        (theme.primary, theme.primary_foreground)
    } else {
        (theme.secondary, theme.secondary_foreground)
    };
    let bubble = div()
        .max_w(px(440.))
        .px_3()
        .py_2()
        .bg(bg)
        .text_color(fg)
        .text_size(px(14.))
        .line_height(px(22.))
        .when(msg.status == Status::Pending, |this| this.opacity(0.7))
        .child(SelectableText::new(
            SharedString::from(format!("text-{}", msg.id)),
            msg.body.clone(),
        ));
    if !msg.outgoing || msg.status == Status::Ok {
        return bubble.into_any_element();
    }
    let error = state(cx).errors.get(&msg.id).cloned();
    let mismatch = error.as_deref().is_some_and(|e| e.contains("当前不是「"));
    if msg.status == Status::Failed && mismatch {
        let contact = contact.clone();
        return v_flex()
            .items_end()
            .gap_1()
            .child(bubble)
            .child(
                h_flex()
                    .max_w(px(440.))
                    .items_start()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme.danger)
                    .child(
                        div()
                            .mt(px(2.))
                            .flex_none()
                            .child(kit::icon(Ic::CircleAlert, 14.)),
                    )
                    .child(div().flex_1().min_w_0().child(error.unwrap_or_default()))
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("edit-{}", msg.id)))
                            .flex_none()
                            .gap_1()
                            .text_color(theme.link)
                            .font_semibold()
                            .cursor_pointer()
                            .child(kit::icon(Ic::Pencil, 12.))
                            .child("编辑")
                            .on_click(move |_, window, cx| {
                                dialogs::edit_contact(contact.clone(), window, cx)
                            }),
                    ),
            )
            .into_any_element();
    }
    let status = match msg.status {
        Status::Pending => h_flex()
            .gap_1()
            .text_size(px(12.))
            .text_color(theme.muted_foreground)
            .child(Spinner::new().with_size(px(14.)))
            .child("发送中"),
        _ => {
            let (view, id) = (view.clone(), msg.id.clone());
            h_flex()
                .gap_1()
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(kit::icon(Ic::CircleAlert, 14.))
                .child("发送失败 ·")
                .child(
                    kit::link_text("重试", cx)
                        .id(SharedString::from(format!("retry-{}", msg.id)))
                        .underline()
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.retry(&id, window, cx));
                        }),
                )
        }
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(status)
        .child(bubble)
        .into_any_element()
}

/// 文件卡片（设计规格 · Attachment）：宽 320（服务器 340）。
fn render_file(
    msg: &Message,
    contact: &Contact,
    local: Option<PathBuf>,
    view: &WeakEntity<ChatView>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let host = contact.kind == ContactKind::Host;
    let width = if host { 340. } else { 320. };
    let size = msg.size.unwrap_or(0);
    let done = state(cx).progress.get(&msg.id).copied().unwrap_or(0);
    let percent = if size == 0 {
        100.
    } else {
        done as f32 * 100. / size as f32
    };
    let meta = match msg.status {
        Status::Pending => format!(
            "{} / {} · {percent:.0}%",
            format_size(done),
            format_size(size)
        ),
        _ => format_size(size),
    };
    let verb = match (host, msg.outgoing) {
        (true, true) => "上传",
        (true, false) => "下载",
        (false, true) => "发送",
        (false, false) => "接收",
    };
    // 联系人发来的文件失败时只能由对方重试；其余失败可在本端重试。
    let can_retry = msg.outgoing || host;
    let existing = local.filter(|p| p.exists());

    v_flex()
        .w(px(width))
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .p_3()
                .gap_3()
                .child(
                    div()
                        .size(px(40.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.muted)
                        .child(kit::icon(kit::file_icon(&msg.body), 20.)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_semibold()
                                .whitespace_nowrap()
                                .child(kit::middle_ellipsis(
                                    &msg.body,
                                    if host { 18.5 } else { 17. },
                                )),
                        )
                        .when_some(msg.remote_path.clone(), |this, remote| {
                            this.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .font_family(kit::mono(cx))
                                    .truncate()
                                    .child(remote),
                            )
                        })
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(meta),
                        ),
                ),
        )
        .when(msg.status == Status::Pending, |this| {
            this.child(
                div()
                    .px_3()
                    .pb_3()
                    .child(kit::progress_bar(percent, 4., cx)),
            )
        })
        // 已完成：「打开」「打开文件夹」各占一半，文字居中，悬停该半块 secondary 底（v0.2 U1）。
        .when(msg.status == Status::Ok && existing.is_some(), |this| {
            let path = existing.clone().unwrap();
            let reveal = path.clone();
            let (open_view, open_id) = (view.clone(), msg.id.clone());
            let (reveal_view, reveal_id) = (view.clone(), msg.id.clone());
            let action = |id: String, label: &'static str| {
                div()
                    .id(SharedString::from(id))
                    .flex_1()
                    .min_w_0()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.secondary))
                    .active(|s| s.bg(theme.secondary))
                    .child(label)
            };
            this.child(
                h_flex()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(theme.link)
                    .child(action(format!("open-{}", msg.id), "打开").on_click(
                        move |_, window, cx| {
                            open_local(&open_view, &open_id, &path, false, window, cx)
                        },
                    ))
                    .child(
                        action(format!("reveal-{}", msg.id), "打开文件夹")
                            .border_l_1()
                            .border_color(theme.border)
                            .on_click(move |_, window, cx| {
                                open_local(&reveal_view, &reveal_id, &reveal, true, window, cx)
                            }),
                    ),
            )
        })
        // 传输成功但本地文件已被移动、改名或删除：在操作行的位置给出提示，与操作行等高（v0.2 #13）。
        .when(msg.status == Status::Ok && existing.is_none(), |this| {
            this.child(
                div().border_t_1().border_color(theme.border).child(
                    h_flex()
                        .h(px(32.))
                        .px_3()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child("文件已被移动或删除"),
                ),
            )
        })
        .when(msg.status == Status::Failed, |this| {
            let (view, id) = (view.clone(), msg.id.clone());
            this.child(
                h_flex()
                    .h(px(32.))
                    .px_3()
                    .gap(px(6.))
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(px(13.))
                    .text_color(theme.danger)
                    .child(kit::icon(Ic::CircleAlert, 14.))
                    .child(if can_retry {
                        format!("{verb}失败 ·")
                    } else {
                        format!("{verb}失败")
                    })
                    .when(can_retry, |this| {
                        this.child(
                            h_flex()
                                .id(SharedString::from(format!("retry-{}", msg.id)))
                                .gap_1()
                                .text_color(theme.link)
                                .font_semibold()
                                .cursor_pointer()
                                .child(kit::icon(Ic::RotateCw, 14.))
                                .child("重试")
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| this.retry(&id, window, cx));
                                }),
                        )
                    }),
            )
        })
}

/// 卡片的「打开」「打开文件夹」与图片点击（v0.2 B1）：失败时弹出错误 Toast，并刷新该消息，
/// 文件刚被删除时立即换成「文件已被移动或删除」。
fn open_local(
    view: &WeakEntity<ChatView>,
    id: &str,
    path: &Path,
    reveal: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let result = if reveal {
        desktop::reveal(path, cx)
    } else {
        desktop::open(path, cx)
    };
    if let Err(e) = result {
        window.push_notification(Notification::error(format!("{e:#}")), cx);
        let _ = view.update(cx, |this, cx| this.refresh_message(id, cx));
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let view = cx.entity().downgrade();
        let auth_failed = state(cx).auth_failed.contains(&self.contact.id);
        let dragging = cx.has_active_drag();
        let scroller = MessageScroller::new(
            SharedString::from(format!("messages-{}", self.contact.id)),
            self.scroller.clone(),
            move |index, _, cx| {
                let Some(this) = view.upgrade() else {
                    return div().into_any_element();
                };
                let this = this.read(cx);
                if index >= this.messages.len() {
                    return div().into_any_element();
                }
                render_row(this, index, &view, cx)
            },
        )
        .with_list_style(StyleRefinement::default().pb_4())
        // 去掉组件默认的行内边距与行间距，组内 4 / 组间 16 由 render_row 控制。
        .with_row_style(StyleRefinement::default().px_0().pb_0())
        .with_jump_button_label("新消息")
        .with_jump_button_renderer(|button| button.icon(kit::icon(Ic::ArrowDown, 14.)));

        let contact = self.contact.clone();
        v_flex()
            .id("chat")
            .size_full()
            .relative()
            .bg(theme.background)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(self.render_header(window, cx))
            .when(auth_failed, |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .px_6()
                        .py(px(10.))
                        .gap_3()
                        .bg(theme.red_light)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(kit::icon(Ic::KeyRound, 16.).text_color(theme.danger))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(14.))
                                .child("无法登录该服务器，访问密钥可能已失效"),
                        )
                        .child(Button::new("reauth").primary().label("重新授权").on_click(
                            move |_, window, cx| {
                                dialogs::server(
                                    dialogs::ServerMode::Reauth(contact.clone()),
                                    window,
                                    cx,
                                )
                            },
                        )),
                )
            })
            .child(div().flex_1().min_h_0().child(scroller))
            .child(self.render_footer(cx))
            .when(dragging, |this| this.child(self.render_drop_overlay(cx)))
    }
}
