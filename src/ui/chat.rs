//! 会话视图：联系人与服务器共用（设计 4.4、5.4、5.5、6）。

use std::path::PathBuf;

use gpui_kit::{
    base::SelectableText,
    component::{
        ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _,
        alert::Alert,
        attachment::{
            Attachment, AttachmentActions, AttachmentContent, AttachmentDescription,
            AttachmentMedia, AttachmentStatus, AttachmentTitle,
        },
        avatar::Avatar,
        bubble::{Bubble, BubbleVariant},
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{InputEvent, Textarea, TextareaState},
        message::{
            Message as MessageRow, MessageAlignment, MessageContent, MessageFooter, MessageHeader,
        },
        message_scroller::{MessageScroller, MessageScrollerState},
        notification::Notification,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{AppState, core, dialogs, format_full_time, format_size, format_time, state};
use crate::{
    data,
    db::{Contact, ContactKind, Message, MsgKind, SameName, Status, now_ms},
    host, peer,
};

pub struct ChatView {
    contact: Contact,
    messages: Vec<Message>,
    scroller: Entity<MessageScrollerState>,
    composer: Option<Entity<TextareaState>>,
    _subscriptions: Vec<Subscription>,
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

        let mut subscriptions = Vec::new();
        let composer = (contact.kind == ContactKind::Peer).then(|| {
            let composer = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("输入消息，Enter 发送，Shift+Enter 换行")
                    .submit_on_enter(true)
                    .auto_grow(1, 6)
            });
            subscriptions.push(
                cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                    if let InputEvent::PressEnter { shift: false, .. } = event {
                        this.send_text(window, cx);
                    }
                }),
            );
            composer
        });
        Self {
            contact,
            messages,
            scroller,
            composer,
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
        match self.messages.iter_mut().find(|m| m.id == msg.id) {
            Some(existing) => *existing = msg,
            None => {
                self.messages.push(msg);
                self.scroller.update(cx, |s, cx| _ = s.append(1, cx));
            }
        }
        cx.notify();
    }

    pub fn on_status(&mut self, id: &str, status: Status, cx: &mut Context<Self>) {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
            msg.status = status;
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

    /// 入库并在后台发送 / 上传。
    fn dispatch(&mut self, msg: Message, window: &mut Window, cx: &mut Context<Self>) {
        let core = core(cx);
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
            window.push_notification("不支持发送文件夹", cx);
        }
        if !files.is_empty() {
            self.confirm_same_name(files, window, cx);
        }
    }

    /// 图片复制到 files/ 并按时间戳命名后发送（设计 3.3）。
    fn send_image_file(&mut self, source: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<Message> {
            let core = core(cx);
            let (path, mut file) = data::create_unique(
                &core.files_dir(),
                &data::image_name(&peer::file_name(&source)),
            )?;
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
            let (path, mut file) =
                data::create_unique(&core.files_dir(), &data::image_name(&format!("x.{ext}")))?;
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

    /// 同名文件确认（设计 6）：只看本工具的成功记录。
    fn confirm_same_name(
        &mut self,
        files: Vec<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let core = core(cx);
        let host = self.contact.kind == ContactKind::Host;
        let conflicts: Vec<String> = files
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
                let verb = if host { "上传" } else { "发送" };
                Some(format!("{name} — 上次{verb}于 {}", format_full_time(ts)))
            })
            .collect();
        if conflicts.is_empty() {
            self.send_files(files, window, cx);
            return;
        }
        let (title, note, ok) = if host {
            (
                "已上传过同名文件",
                format!(
                    "继续上传将覆盖服务器上 {} 中的同名文件。",
                    self.contact.remote_dir
                ),
                "继续上传",
            )
        } else {
            (
                "已发送过同名文件",
                "继续发送后，对方会再收到一份。".to_string(),
                "继续发送",
            )
        };
        let description = format!("{}\n\n{note}", conflicts.join("\n"));
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, files) = (view.clone(), files.clone());
            alert
                .title(title)
                .description(description.clone())
                .show_cancel(true)
                .cancel_text("取消")
                .ok_text(ok)
                .on_ok(move |_, window, cx| {
                    let files = files.clone();
                    let _ = view.update(cx, |this, cx| this.send_files(files, window, cx));
                    true
                })
        });
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

    // ------------------------------------------------------------ 渲染

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &self.contact;
        let title = h_flex()
            .gap_2()
            .items_baseline()
            .child(div().font_semibold().child(c.name.clone()));
        match c.kind {
            ContactKind::Peer => h_flex()
                .gap_3()
                .child(Avatar::new().name(c.name.clone()).small())
                .child(
                    title.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(c.host.clone()),
                    ),
                ),
            ContactKind::Host => h_flex()
                .w_full()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            title.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{}@{}:{}", c.user, c.host, c.port)),
                            ),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("远程目录：{}", c.remote_dir))
                                .child(
                                    Button::new("change-dir")
                                        .link()
                                        .small()
                                        .label("更改")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_remote_dir(window, cx)
                                        })),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("download")
                                .icon(Icon::new(gpui_kit::assets::IconName::Download))
                                .label("下载文件")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pick_downloads(window, cx)
                                })),
                        )
                        .child(
                            Button::new("terminal")
                                .icon(IconName::SquareTerminal)
                                .label("在终端中打开")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_terminal(window, cx)
                                })),
                        ),
                ),
        }
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.composer {
            Some(composer) => {
                let paste_view = cx.entity().downgrade();
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        Button::new("attach")
                            .ghost()
                            .icon(Icon::new(gpui_kit::assets::IconName::Paperclip))
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
                            .icon(Icon::new(gpui_kit::assets::IconName::SendHorizontal))
                            .tooltip("发送")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_text(window, cx)),
                            ),
                    )
                    .into_any_element()
            }
            None => h_flex()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .p_3()
                        .border_1()
                        .border_dashed()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "拖拽文件到此处，上传到 {}",
                            self.contact.remote_dir
                        )),
                )
                .child(
                    Button::new("upload")
                        .primary()
                        .icon(Icon::new(gpui_kit::assets::IconName::Upload))
                        .label("上传文件")
                        .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx))),
                )
                .into_any_element(),
        }
    }
}

/// 渲染一行消息。只读取数据，不持有视图的可变借用。
fn render_row(
    msg: &Message,
    contact: &Contact,
    view: &WeakEntity<ChatView>,
    cx: &App,
) -> AnyElement {
    let core = &state(cx).core;
    let progress = state(cx).progress.get(&msg.id).copied();
    let alignment = if msg.outgoing {
        MessageAlignment::End
    } else {
        MessageAlignment::Start
    };
    let mut row = MessageRow::new()
        .id(SharedString::from(format!("msg-{}", msg.id)))
        .alignment(alignment)
        .header(MessageHeader::new().child(format_time(msg.ts)));
    if !msg.outgoing {
        row = match contact.kind {
            ContactKind::Peer => row.avatar(Avatar::new().name(contact.name.clone()).small()),
            ContactKind::Host => row.avatar(Icon::new(gpui_kit::assets::IconName::Server)),
        };
    }

    let local = msg.path.as_deref().map(|p| core.resolve(p));
    let content = match msg.kind {
        MsgKind::Text => {
            let bubble = Bubble::new()
                .when(!msg.outgoing, |b| b.with_variant(BubbleVariant::Secondary))
                .child(SelectableText::new(
                    SharedString::from(format!("text-{}", msg.id)),
                    msg.body.clone(),
                ));
            MessageContent::new().bubble(bubble)
        }
        MsgKind::Image
            if msg.status == Status::Ok && local.as_ref().is_some_and(|p| p.exists()) =>
        {
            let path = local.clone().unwrap();
            MessageContent::new().child(
                div()
                    .id(SharedString::from(format!("img-{}", msg.id)))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_with_system(&path))
                    .child(
                        img(local.clone().unwrap())
                            .max_w(px(240.))
                            .max_h(px(240.))
                            .object_fit(ObjectFit::Contain)
                            .rounded(cx.theme().radius),
                    ),
            )
        }
        _ => MessageContent::new().child(render_file(msg, contact, local, progress, view)),
    };
    row = row.content(content);

    if msg.kind == MsgKind::Text && msg.status != Status::Ok {
        let footer = match msg.status {
            Status::Pending => MessageFooter::new().child("发送中…"),
            _ => {
                let (view, id) = (view.clone(), msg.id.clone());
                MessageFooter::new().child(
                    h_flex()
                        .gap_1()
                        .text_color(cx.theme().danger)
                        .child("发送失败 ·")
                        .child(
                            Button::new(SharedString::from(format!("retry-{}", msg.id)))
                                .link()
                                .small()
                                .label("重试")
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| this.retry(&id, window, cx));
                                }),
                        ),
                )
            }
        };
        row = row.footer(footer);
    }
    row.into_any_element()
}

fn render_file(
    msg: &Message,
    contact: &Contact,
    local: Option<PathBuf>,
    progress: Option<u64>,
    view: &WeakEntity<ChatView>,
) -> AnyElement {
    let size = msg.size.unwrap_or(0);
    let status = match msg.status {
        Status::Pending => AttachmentStatus::Uploading,
        Status::Ok => AttachmentStatus::Complete,
        Status::Failed => AttachmentStatus::Failed,
    };
    let percent = progress.map(|done| {
        if size == 0 {
            100.0
        } else {
            done as f32 * 100.0 / size as f32
        }
    });
    let mut description = match msg.status {
        Status::Pending => format!("{:.0}% · {}", percent.unwrap_or(0.0), format_size(size)),
        Status::Ok => format_size(size),
        Status::Failed => {
            let verb = match (contact.kind, msg.outgoing) {
                (ContactKind::Host, true) => "上传失败",
                (ContactKind::Host, false) => "下载失败",
                (ContactKind::Peer, true) => "发送失败",
                (ContactKind::Peer, false) => "接收失败",
            };
            format!("{verb} · {}", format_size(size))
        }
    };
    if let Some(remote) = &msg.remote_path {
        description = format!("{description} · {remote}");
    }
    let icon = if msg.kind == MsgKind::Image {
        IconName::GalleryVerticalEnd
    } else {
        IconName::FileText
    };

    let mut actions = AttachmentActions::new();
    if msg.status == Status::Ok
        && let Some(path) = local.filter(|p| p.exists())
    {
        let reveal = path.clone();
        actions = actions
            .child(
                Button::new(SharedString::from(format!("open-{}", msg.id)))
                    .ghost()
                    .xsmall()
                    .label("打开")
                    .on_click(move |_, _, cx| cx.open_with_system(&path)),
            )
            .child(
                Button::new(SharedString::from(format!("reveal-{}", msg.id)))
                    .ghost()
                    .xsmall()
                    .label("打开所在文件夹")
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
            );
    }
    // 联系人发来的文件失败时只能由对方重试；其余失败可在本端重试。
    let can_retry =
        msg.status == Status::Failed && (msg.outgoing || contact.kind == ContactKind::Host);
    let mut card = Attachment::new()
        .id(SharedString::from(format!("file-{}", msg.id)))
        .w(px(360.))
        .status(status)
        .media(AttachmentMedia::new().child(Icon::new(icon)))
        .content(
            AttachmentContent::new()
                .title(AttachmentTitle::new(msg.body.clone()))
                .description(AttachmentDescription::new(description)),
        )
        .actions(actions);
    if let Some(percent) = percent {
        card = card.progress(percent);
    }
    if can_retry {
        let (view, id) = (view.clone(), msg.id.clone());
        card = card.on_retry(move |_, window, cx| {
            let _ = view.update(cx, |this, cx| this.retry(&id, window, cx));
        });
    }
    card.into_any_element()
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let auth_failed = state(cx).auth_failed.contains(&self.contact.id);
        let scroller = MessageScroller::new(
            SharedString::from(format!("messages-{}", self.contact.id)),
            self.scroller.clone(),
            move |index, _, cx| {
                let Some(this) = view.upgrade() else {
                    return div().into_any_element();
                };
                let this = this.read(cx);
                match this.messages.get(index) {
                    Some(msg) => render_row(msg, &this.contact, &view, cx),
                    None => div().into_any_element(),
                }
            },
        )
        .with_list_style(StyleRefinement::default().px_4().py_3())
        .with_jump_button_label("新消息")
        .with_bottom_fade(cx.theme().background);

        let contact = self.contact.clone();
        v_flex()
            .id("chat")
            .size_full()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_paths(paths.paths().to_vec(), window, cx);
            }))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_header(cx)),
            )
            .when(auth_failed, |this| {
                this.child(
                    h_flex()
                        .px_4()
                        .pt_3()
                        .gap_2()
                        .child(div().flex_1().child(Alert::warning(
                            "auth-failed",
                            "无法登录该服务器，访问密钥可能已失效",
                        )))
                        .child(Button::new("reauth").label("重新授权").on_click(
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
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(self.render_footer(cx)),
            )
    }
}
