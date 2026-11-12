//! 主窗口：标题栏 + 侧边栏 + 会话区（设计 01 / 02，docs/ui「LE Window」）。

use gpui_kit::{
    assets::IconName as Ic,
    component::{
        ActiveTheme as _, StyledExt as _, Theme, TitleBar, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::{ContextMenuExt as _, PopupMenuItem},
        notification::Notification,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{
    AppState, chat::ChatView, core, dialogs, kit, refresh_tray, settings::SettingsView, state,
    window_closed,
};
use crate::db::{Contact, ContactKind, Message, Status};

enum Page {
    Empty,
    Chat(Entity<ChatView>),
    Settings(Entity<SettingsView>),
}

pub struct AppView {
    contacts: Vec<Contact>,
    page: Page,
    _appearance: Subscription,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release(|_, cx| window_closed(cx)).detach();
        // 跟随系统明暗切换浅色 / 深色主题。
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
        });
        let mut this = Self {
            contacts: Vec::new(),
            page: Page::Empty,
            _appearance: appearance,
        };
        this.reload_contacts(cx);
        if let Some(id) = state(cx).selected {
            this.select(id, window, cx);
        }
        this
    }

    pub fn reload_contacts(&mut self, cx: &mut Context<Self>) {
        self.contacts = core(cx).db(|db| db.contacts()).unwrap_or_default();
        if let Page::Chat(chat) = &self.page {
            let id = chat.read(cx).contact_id();
            match self.contacts.iter().find(|c| c.id == id) {
                Some(contact) => {
                    let contact = contact.clone();
                    chat.update(cx, |chat, cx| chat.set_contact(contact, cx));
                }
                None => self.page = Page::Empty,
            }
        }
        cx.notify();
    }

    pub fn select(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(contact) = self.contacts.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let app = cx.global_mut::<AppState>();
        app.selected = Some(id);
        app.unread.remove(&id);
        refresh_tray(cx);
        self.page = Page::Chat(cx.new(|cx| ChatView::new(contact, window, cx)));
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.global_mut::<AppState>().selected = None;
        self.page = Page::Settings(cx.new(|cx| SettingsView::new(window, cx)));
        cx.notify();
    }

    pub fn on_message(&mut self, msg: Message, window: &mut Window, cx: &mut Context<Self>) {
        if let Page::Chat(chat) = &self.page {
            chat.update(cx, |chat, cx| chat.on_message(msg, window, cx));
        }
        self.reload_contacts(cx);
    }

    pub fn on_status(&mut self, id: &str, status: Status, cx: &mut Context<Self>) {
        if let Page::Chat(chat) = &self.page {
            chat.update(cx, |chat, cx| chat.on_status(id, status, cx));
        }
    }

    pub fn refresh_chat(&mut self, cx: &mut Context<Self>) {
        if let Page::Chat(chat) = &self.page {
            chat.update(cx, |_, cx| cx.notify());
        }
    }

    fn delete_contact(&mut self, contact: Contact, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        let confirm = kit::Confirm {
            title: format!("删除「{}」？", contact.name),
            path: None,
            body: "聊天记录将被删除，已传输的文件会保留。".into(),
            ok: "删除",
            danger: true,
        };
        kit::open_confirm(
            confirm,
            move |window, cx| {
                let core = core(cx);
                if let Err(e) = core.db(|db| db.delete_contact(contact.id)) {
                    window.push_notification(Notification::error(format!("{e:#}")), cx);
                    return;
                }
                let app = cx.global_mut::<AppState>();
                app.unread.remove(&contact.id);
                app.auth_failed.remove(&contact.id);
                if app.selected == Some(contact.id) {
                    app.selected = None;
                }
                refresh_tray(cx);
                let (forget, id) = (core.clone(), contact.id);
                core.runtime
                    .spawn(async move { crate::host::forget(&forget, id).await });
                let _ = view.update(cx, |view, cx| view.reload_contacts(cx));
            },
            window,
            cx,
        );
    }

    // ------------------------------------------------------------ 侧边栏

    fn group_header(
        &self,
        label: &'static str,
        add_server: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .h(px(32.))
            .pl_2()
            .pr_1()
            .text_size(px(12.))
            .font_semibold()
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_1().child(label))
            .child(
                Button::new(SharedString::from(format!("add-{label}")))
                    .ghost()
                    .size(px(24.))
                    .icon(kit::icon(Ic::Plus, 16.))
                    .text_color(cx.theme().foreground)
                    .tooltip(if add_server {
                        "添加服务器"
                    } else {
                        "添加联系人"
                    })
                    .on_click(cx.listener(move |_, _, window, cx| {
                        if add_server {
                            dialogs::server(dialogs::ServerMode::Add, window, cx)
                        } else {
                            dialogs::add_contact(window, cx)
                        }
                    })),
            )
    }

    fn contact_row(&self, contact: &Contact, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected =
            matches!(&self.page, Page::Chat(chat) if chat.read(cx).contact_id() == contact.id);
        let unread = state(cx).unread.get(&contact.id).copied().unwrap_or(0);
        let (avatar, secondary) = match contact.kind {
            ContactKind::Peer => (kit::avatar_text(&contact.name, cx), contact.host.clone()),
            ContactKind::Host => {
                let port = if contact.port == 22 {
                    String::new()
                } else {
                    format!(":{}", contact.port)
                };
                (
                    kit::avatar_server(cx),
                    format!("{}@{}{port}", contact.user, contact.host),
                )
            }
        };
        let id = contact.id;
        let view = cx.entity().downgrade();
        let for_menu = contact.clone();
        h_flex()
            .id(SharedString::from(format!("contact-{id}")))
            .h(px(52.))
            .px_2()
            .gap_3()
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.list_active))
            .when(!selected, |this| this.hover(|s| s.bg(theme.sidebar_accent)))
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_semibold()
                            .truncate()
                            .child(contact.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(secondary),
                    ),
            )
            .when(unread > 0, |this| this.child(kit::badge(unread, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
            .context_menu(move |menu, _, cx| {
                let (edit, delete) = (for_menu.clone(), for_menu.clone());
                let (view_edit, view_delete) = (view.clone(), view.clone());
                let danger = cx.theme().danger;
                menu.min_w(px(160.))
                    .item(
                        PopupMenuItem::element(|_, _| {
                            h_flex()
                                .gap_2()
                                .child(kit::icon(Ic::Pencil, 16.))
                                .child("编辑")
                        })
                        .on_click(move |_, window, cx| {
                            let contact = edit.clone();
                            let _ = view_edit.update(cx, |_, cx| match contact.kind {
                                ContactKind::Peer => dialogs::edit_contact(contact, window, cx),
                                ContactKind::Host => {
                                    dialogs::server(dialogs::ServerMode::Edit(contact), window, cx)
                                }
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::element(move |_, _| {
                            h_flex()
                                .gap_2()
                                .text_color(danger)
                                .child(kit::icon(Ic::Trash, 16.))
                                .child("删除")
                        })
                        .on_click(move |_, window, cx| {
                            let contact = delete.clone();
                            let _ = view_delete
                                .update(cx, |view, cx| view.delete_contact(contact, window, cx));
                        }),
                    )
            })
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let peers: Vec<_> = self
            .contacts
            .iter()
            .filter(|c| c.kind == ContactKind::Peer)
            .cloned()
            .collect();
        let hosts: Vec<_> = self
            .contacts
            .iter()
            .filter(|c| c.kind == ContactKind::Host)
            .cloned()
            .collect();
        let empty_hint = |text: &'static str, bottom: f32| {
            div()
                .px_2()
                .pt_1()
                .pb(px(bottom))
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let peer_rows: Vec<_> = peers
            .iter()
            .map(|c| self.contact_row(c, cx).into_any_element())
            .collect();
        let host_rows: Vec<_> = hosts
            .iter()
            .map(|c| self.contact_row(c, cx).into_any_element())
            .collect();
        let settings_active = matches!(self.page, Page::Settings(_));
        v_flex()
            .w(px(260.))
            .flex_none()
            .h_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            // v0.2 U2：本机 IP 移到设置页，侧边栏直接从「联系人」分组开始。
            .child(
                v_flex()
                    .id("sidebar-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .child(self.group_header("联系人", false, cx))
                    .when(peers.is_empty(), |this| {
                        this.child(empty_hint("点击「+」，输入对方 IP 即可添加。对方需已运行 Local Echo。", 16.))
                    })
                    .children(peer_rows)
                    .child(div().mt_2().child(self.group_header("服务器", true, cx)))
                    .when(hosts.is_empty(), |this| {
                        this.child(empty_hint(
                            "点击「+」，输入地址、用户名和密码添加一次。之后上传、下载和打开终端都无需再输入密码。",
                            4.,
                        ))
                    })
                    .children(host_rows),
            )
            .child(
                // 底栏高度 60（v0.2 U3）：「设置」行 36 垂直居中，与右侧输入区 / 服务器底栏的上边框对齐。
                v_flex()
                    .h(kit::footer_height())
                    .flex_none()
                    .px(px(8.))
                    .justify_center()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(
                    h_flex()
                        .id("settings")
                        .h(px(36.))
                        .px_2()
                        .gap_3()
                        .text_size(px(14.))
                        .cursor_pointer()
                        .when(settings_active, |this| this.bg(theme.list_active))
                        .when(!settings_active, |this| this.hover(|s| s.bg(theme.sidebar_accent)))
                        .child(kit::icon(Ic::Settings, 16.))
                        .child("设置")
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                ),
            )
    }

    fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = if self.contacts.is_empty() {
            "在局域网内直接收发消息和文件，不经过任何服务器。"
        } else {
            "从左侧选择一个会话，或添加新的联系人和服务器。"
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .child(
                v_flex()
                    .w(px(360.))
                    .gap_4()
                    .child(kit::mark(40.).text_color(cx.theme().primary))
                    .child(
                        div()
                            .text_size(px(20.))
                            .line_height(px(28.))
                            .font_semibold()
                            .child(heading),
                    )
                    .child(
                        h_flex()
                            .mt_2()
                            .gap_2()
                            .child(
                                Button::new("home-add-contact")
                                    .primary()
                                    .icon(kit::icon(Ic::UserPlus, 16.))
                                    .label("添加联系人")
                                    .on_click(cx.listener(|_, _, window, cx| {
                                        dialogs::add_contact(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("home-add-server")
                                    .secondary()
                                    .icon(kit::icon(Ic::Server, 16.))
                                    .label("添加服务器")
                                    .on_click(cx.listener(|_, _, window, cx| {
                                        dialogs::server(dialogs::ServerMode::Add, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match &self.page {
            Page::Empty => self.render_home(cx).into_any_element(),
            Page::Chat(chat) => chat.clone().into_any_element(),
            Page::Settings(settings) => settings.clone().into_any_element(),
        };
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new()
                    .h(px(36.))
                    .bg(theme.title_bar)
                    .border_color(theme.title_bar_border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(kit::mark(16.).text_color(theme.foreground))
                            .child(div().text_size(px(13.)).font_semibold().child("Local Echo")),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(theme.background)
                            .child(page),
                    ),
            )
    }
}
