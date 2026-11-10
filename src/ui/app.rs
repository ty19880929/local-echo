//! 主窗口：标题栏 + 侧边栏 + 会话区（设计 9）。

use std::sync::Arc;

use gpui_kit::{
    component::{
        ActiveTheme as _, Icon, IconName, Selectable as _, TitleBar, WindowExt as _,
        button::{Button, ButtonVariant, ButtonVariants as _},
        h_flex,
        menu::PopupMenuItem,
        notification::Notification,
        sidebar::{
            Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
        },
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{
    AppState, chat::ChatView, core, dialogs, refresh_tray, settings::SettingsView, state,
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
    ips: Vec<String>,
    page: Page,
}

pub fn logo() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/tray.png").to_vec(),
    ))
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release(|_, cx| window_closed(cx)).detach();
        let ips = local_ip_address::list_afinet_netifas()
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, ip)| ip.is_ipv4() && !ip.is_loopback())
            .map(|(_, ip)| ip.to_string())
            .collect();
        let mut this = Self {
            contacts: Vec::new(),
            ips,
            page: Page::Empty,
        };
        this.reload_contacts(cx);
        // 窗口重建时恢复之前选中的会话。
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
        // 会话列表按最近消息排序。
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
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let contact = contact.clone();
            alert
                .title(format!("删除「{}」？", contact.name))
                .description("聊天记录将被删除，已传输的文件会保留。")
                .show_cancel(true)
                .cancel_text("取消")
                .ok_text("删除")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let core = core(cx);
                    if let Err(e) = core.db(|db| db.delete_contact(contact.id)) {
                        window.push_notification(Notification::error(format!("{e:#}")), cx);
                        return true;
                    }
                    let app = cx.global_mut::<AppState>();
                    app.unread.remove(&contact.id);
                    app.auth_failed.remove(&contact.id);
                    if app.selected == Some(contact.id) {
                        app.selected = None;
                    }
                    refresh_tray(cx);
                    let id = contact.id;
                    let forget = core.clone();
                    core.runtime
                        .spawn(async move { crate::host::forget(&forget, id).await });
                    let _ = view.update(cx, |view, cx| view.reload_contacts(cx));
                    true
                })
        });
    }

    fn contact_item(&self, contact: &Contact, cx: &mut Context<Self>) -> SidebarMenuItem {
        let selected =
            matches!(&self.page, Page::Chat(chat) if chat.read(cx).contact_id() == contact.id);
        let unread = state(cx).unread.get(&contact.id).copied().unwrap_or(0);
        let (label, icon) = match contact.kind {
            ContactKind::Peer => (
                format!("{} · {}", contact.name, contact.host),
                Icon::new(IconName::User),
            ),
            ContactKind::Host => (
                format!("{} · {}@{}", contact.name, contact.user, contact.host),
                Icon::new(gpui_kit::assets::IconName::Server),
            ),
        };
        let id = contact.id;
        let view = cx.entity().downgrade();
        let for_menu = contact.clone();
        SidebarMenuItem::new(label)
            .icon(icon)
            .active(selected)
            .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
            .when(unread > 0, |item| {
                item.suffix(move |_, cx| {
                    div()
                        .px_1p5()
                        .min_w_5()
                        .rounded_full()
                        .bg(cx.theme().danger)
                        .text_color(cx.theme().danger_foreground)
                        .text_xs()
                        .text_center()
                        .child(unread.to_string())
                })
            })
            .context_menu(move |menu, _, _| {
                let (edit, delete) = (for_menu.clone(), for_menu.clone());
                let (view_edit, view_delete) = (view.clone(), view.clone());
                menu.item(PopupMenuItem::new("编辑").on_click(move |_, window, cx| {
                    let contact = edit.clone();
                    let _ = view_edit.update(cx, |_, cx| match contact.kind {
                        ContactKind::Peer => dialogs::edit_contact(contact, window, cx),
                        ContactKind::Host => {
                            dialogs::server(dialogs::ServerMode::Edit(contact), window, cx)
                        }
                    });
                }))
                .item(PopupMenuItem::new("删除").on_click(move |_, window, cx| {
                    let contact = delete.clone();
                    let _ =
                        view_delete.update(cx, |view, cx| view.delete_contact(contact, window, cx));
                }))
            })
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let add_contact = SidebarMenuItem::new("添加联系人")
            .icon(IconName::Plus)
            .on_click(cx.listener(|_, _, window, cx| dialogs::add_contact(window, cx)));
        let add_server = SidebarMenuItem::new("添加服务器")
            .icon(IconName::Plus)
            .on_click(cx.listener(|_, _, window, cx| {
                dialogs::server(dialogs::ServerMode::Add, window, cx)
            }));
        let mut peer_items: Vec<_> = peers.iter().map(|c| self.contact_item(c, cx)).collect();
        peer_items.push(add_contact);
        let mut host_items: Vec<_> = hosts.iter().map(|c| self.contact_item(c, cx)).collect();
        host_items.push(add_server);

        let ips = if self.ips.is_empty() {
            "未连接网络".to_string()
        } else {
            self.ips.join(" / ")
        };
        let settings_active = matches!(self.page, Page::Settings(_));
        Sidebar::new("sidebar")
            .w(px(260.))
            .header(
                SidebarHeader::new().child(
                    div()
                        .id("my-ip")
                        .w_full()
                        .cursor_pointer()
                        .child(
                            v_flex()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("本机 IP"),
                                )
                                .child(div().text_sm().child(ips.clone())),
                        )
                        .tooltip({
                            let tip = SharedString::from("点击复制");
                            move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                    .build(window, cx)
                            }
                        })
                        .on_click(move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(ips.clone()));
                            window.push_notification("已复制本机 IP", cx);
                        }),
                ),
            )
            .child(SidebarGroup::new("联系人").child(SidebarMenu::new().children(peer_items)))
            .child(SidebarGroup::new("服务器").child(SidebarMenu::new().children(host_items)))
            .footer(
                SidebarFooter::new().child(
                    Button::new("settings")
                        .ghost()
                        .w_full()
                        .justify_start()
                        .icon(IconName::Settings)
                        .label("设置")
                        .selected(settings_active)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                ),
            )
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(img(logo()).size(px(64.)))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("在局域网内与联系人直接聊天、传文件，或与服务器互传文件"),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("empty-add-contact")
                            .primary()
                            .label("添加联系人")
                            .on_click(
                                cx.listener(|_, _, window, cx| dialogs::add_contact(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("empty-add-server")
                            .label("添加服务器")
                            .on_click(cx.listener(|_, _, window, cx| {
                                dialogs::server(dialogs::ServerMode::Add, window, cx)
                            })),
                    ),
            )
    }
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match &self.page {
            Page::Empty => self.render_empty(cx).into_any_element(),
            Page::Chat(chat) => chat.clone().into_any_element(),
            Page::Settings(settings) => settings.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .gap_2()
                        .child(img(logo()).size(px(18.)))
                        .child(div().text_sm().child("Local Echo")),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(div().flex_1().min_w_0().h_full().child(page)),
            )
    }
}
