//! 设置页：本机 IP、数据目录与迁移、开机自动启动、关于、退出（设计 7、8.4；视觉见 docs/ui 09、v0.2 02）。

use std::{path::PathBuf, time::Duration};

use gpui_kit::{
    assets::IconName as Ic,
    component::{
        ActiveTheme as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        notification::Notification,
        switch::Switch,
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{AppState, core, format_size, kit, state};
use crate::{
    data, desktop,
    net::{self, LocalIp},
};

/// 侧边栏宽度（含右边框），用于由窗口宽度推算会话区宽度。
const SIDEBAR_W: f32 = 260.;
/// 本机 IP 表格「IP 地址」「操作」两列的宽度（v0.2 规格）。
const IP_COL_W: f32 = 144.;
const ACTION_COL_W: f32 = 40.;

pub struct SettingsView {
    autostart: bool,
    /// 点击「更改…」时有传输正在进行（设计 09-B）。
    blocked: bool,
    /// 本机 IP：每次打开设置页、每次展开时重新读取（v0.2 U2）。
    ips: Vec<LocalIp>,
    /// 刚复制的 IP，2 秒内显示对勾。
    copied: Option<String>,
}

/// 迁移进度对话框的内容（设计 09-E）。
struct MigrationProgress {
    done: u64,
    total: u64,
}

impl Render for MigrationProgress {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let percent = if self.total == 0 {
            0.
        } else {
            self.done as f32 * 100. / self.total as f32
        };
        v_flex()
            .child(
                div()
                    .px_6()
                    .pt(px(20.))
                    .text_size(px(16.))
                    .line_height(px(22.))
                    .font_semibold()
                    .child("正在迁移数据"),
            )
            .child(
                v_flex()
                    .px_6()
                    .pt_3()
                    .pb(px(20.))
                    .gap_3()
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child("完成后应用会自动重启。"),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(kit::progress_bar(percent, 8., cx))
                            .child(
                                h_flex()
                                    .text_size(px(13.))
                                    .child(div().flex_1().child(format!(
                                        "正在移动 {} / {}",
                                        format_size(self.done),
                                        format_size(self.total)
                                    )))
                                    .child(
                                        div()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("{percent:.0}%")),
                                    ),
                            ),
                    ),
            )
    }
}

impl SettingsView {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            autostart: desktop::autostart_enabled(),
            blocked: false,
            ips: net::local_ips(),
            copied: None,
        }
    }

    fn toggle_ips(&mut self, cx: &mut Context<Self>) {
        let app = cx.global_mut::<AppState>();
        app.ips_collapsed = !app.ips_collapsed;
        if !app.ips_collapsed {
            self.ips = net::local_ips();
        }
        cx.notify();
    }

    fn copy_ip(&mut self, ip: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(ip.clone()));
        window.push_notification(Notification::success(format!("已复制 {ip}")), cx);
        self.copied = Some(ip.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied.as_deref() == Some(ip.as_str()) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 「本机 IP」分组（v0.2 规格 · Collapsible + Table）：标题行 28 整行可点 + chevron；
    /// 展开时卡片内通栏表格，收起时一行摘要。
    fn render_ips(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let collapsed = state(cx).ips_collapsed;
        let header = h_flex()
            .id("ips-header")
            .h(px(28.))
            .ml(px(-8.))
            .mr(px(-8.))
            .px_2()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.bg(theme.secondary))
            .child(
                div()
                    .flex_1()
                    .text_size(px(14.))
                    .font_semibold()
                    .child("本机 IP"),
            )
            .child(
                kit::icon(
                    if collapsed {
                        Ic::ChevronRight
                    } else {
                        Ic::ChevronDown
                    },
                    16.,
                )
                .text_color(theme.muted_foreground),
            )
            .on_click(cx.listener(|this, _, _, cx| this.toggle_ips(cx)));
        let card = div()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border);
        let empty = || {
            h_flex()
                .h(px(52.))
                .px_4()
                .gap_2()
                .text_size(px(14.))
                .text_color(theme.muted_foreground)
                .child(kit::icon(Ic::WifiOff, 16.))
                .child("未检测到可用的网络连接")
        };
        let body = match (collapsed, self.ips.first()) {
            (_, None) => card.child(empty()),
            (true, Some(first)) => card.child(
                h_flex()
                    .p_4()
                    .gap_1()
                    .line_height(px(20.))
                    .text_size(px(13.))
                    .child(div().font_family(kit::mono(cx)).child(first.ip.clone()))
                    .when(self.ips.len() > 1, |this| {
                        this.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(format!("等 {} 个", self.ips.len())),
                        )
                    }),
            ),
            (false, Some(_)) => {
                // 网卡名单行中间省略：按会话区宽度推算网卡列可用宽度（14 px 字号）。
                let session = window.viewport_size().width.as_f32() - SIDEBAR_W;
                let card_w = session.min(640.) - 48.;
                let nic_w = card_w - 2. - 32. - 24. - IP_COL_W - ACTION_COL_W;
                let max_em = (nic_w / 14.).max(4.);
                card.child(
                    v_flex()
                        .child(
                            kit::table_head(cx)
                                .px_4()
                                .child(div().flex_1().min_w_0().child("网卡"))
                                .child(div().w(px(IP_COL_W)).child("IP 地址"))
                                .child(
                                    div()
                                        .w(px(ACTION_COL_W))
                                        .flex()
                                        .justify_center()
                                        .child("操作"),
                                ),
                        )
                        .children({
                            let mut rows = Vec::with_capacity(self.ips.len());
                            for (i, ip) in self.ips.iter().enumerate() {
                                rows.push(self.render_ip_row(i, ip, max_em, cx));
                            }
                            rows
                        })
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .border_t_1()
                                .border_color(theme.border)
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "联系人添加你时，填写其中任意一个 IP 即可（端口固定为 {}）",
                                    data::PORT
                                )),
                        ),
                )
            }
        };
        v_flex().gap_1().child(header).child(body)
    }

    fn render_ip_row(
        &self,
        index: usize,
        entry: &LocalIp,
        max_em: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let copied = self.copied.as_deref() == Some(entry.ip.as_str());
        let shown = kit::middle_ellipsis(&entry.nic, max_em);
        let truncated = shown != entry.nic;
        let full = SharedString::from(entry.nic.clone());
        let (ip_click, ip_button) = (entry.ip.clone(), entry.ip.clone());
        kit::table_row(cx)
            .px_4()
            .border_t_1()
            .child(
                div()
                    .id(("ip-nic", index))
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(shown)
                    .when(truncated, |this| {
                        this.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                    }),
            )
            .child(
                // 双击 IP 也可复制。
                div()
                    .id(("ip-addr", index))
                    .w(px(IP_COL_W))
                    .font_family(kit::mono(cx))
                    .text_size(px(13.))
                    .child(entry.ip.clone())
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.copy_ip(ip_click.clone(), window, cx);
                        }
                    })),
            )
            .child(
                div().w(px(ACTION_COL_W)).flex().justify_center().child(
                    div()
                        .id(("ip-copy", index))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(copied, |this| this.bg(theme.secondary))
                        .hover(|s| s.bg(theme.secondary))
                        .text_color(if copied {
                            theme.success
                        } else {
                            theme.muted_foreground
                        })
                        .child(kit::icon(if copied { Ic::Check } else { Ic::Copy }, 16.))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.copy_ip(ip_button.clone(), window, cx)
                        })),
                ),
            )
    }

    fn change_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 有传输正在进行时不能迁移，提示直接显示在数据目录卡片内。
        self.blocked = core(cx).db(|db| db.has_pending()).unwrap_or(false);
        cx.notify();
        if self.blocked {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择数据目录".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(target) = paths.pop() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.confirm_target(target, window, cx)
            });
        })
        .detach();
    }

    fn confirm_target(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if target == core(cx).root {
            return;
        }
        if data::has_data(&target) {
            let confirm = kit::Confirm {
                title: "使用该目录中的已有数据？".into(),
                path: Some(target.display().to_string()),
                body: "当前数据将保留在原位置。切换后应用会自动重启。".into(),
                ok: "使用已有数据",
                danger: false,
            };
            kit::open_confirm(
                confirm,
                move |window, cx| match data::set_root(&target) {
                    Ok(()) => cx.restart(),
                    Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                },
                window,
                cx,
            );
            return;
        }
        let view = cx.entity().downgrade();
        let confirm = kit::Confirm {
            title: format!("将全部数据移动到 {}？", target.display()),
            path: None,
            body: "移动完成后应用会自动重启。".into(),
            ok: "移动",
            danger: false,
        };
        kit::open_confirm(
            confirm,
            move |window, cx| {
                let target = target.clone();
                let _ = view.update(cx, |this, cx| this.migrate(target, window, cx));
            },
            window,
            cx,
        );
    }

    /// 关闭数据库 → 移动数据 → 改写指针文件 → 重启（设计 7.2）。
    fn migrate(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let core = core(cx);
        core.close_db();
        let progress = cx.new(|_| MigrationProgress { done: 0, total: 0 });
        let dialog_content = progress.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            kit::dialog_shell(dialog, 420., cx)
                .overlay_closable(false)
                .keyboard(false)
                .on_ok(|_, _, _| false)
                .child(dialog_content.clone())
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let from = core.root.clone();
        let task = core.runtime.spawn_blocking(move || {
            data::migrate(&from, &target, |done, total| {
                let _ = tx.send((done, total));
            })?;
            data::set_root(&target)
        });
        cx.spawn_in(window, async move |_, cx| {
            while let Some((done, total)) = rx.recv().await {
                progress.update(cx, |p, cx| {
                    (p.done, p.total) = (done, total);
                    cx.notify();
                });
            }
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = cx.update(|window, cx| match result {
                Ok(()) => cx.restart(),
                Err(e) => {
                    // 迁移失败：数据可能已部分移动，重启后按指针文件重新打开。
                    window.close_dialog(cx);
                    window.push_notification(Notification::error(format!("迁移失败：{e:#}")), cx);
                }
            });
        })
        .detach();
    }
}

/// 分组：标题 14/600，下方 8；卡片 popover 底，1 px 边框，内边距 16。
fn section(title: &str, card: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(
            div()
                .text_size(px(14.))
                .font_semibold()
                .child(title.to_string()),
        )
        .child(
            div()
                .p_4()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .child(card),
        )
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let root = core(cx).root.clone();
        let data_dir = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(kit::mono(cx))
                            .text_size(px(13.))
                            .child(root.display().to_string()),
                    )
                    .child(Button::new("open-dir").secondary().label("打开").on_click({
                        let root = root.clone();
                        move |_, _, cx| cx.open_with_system(&root)
                    }))
                    .child(
                        Button::new("change-dir")
                            .secondary()
                            .label("更改…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.change_dir(window, cx)),
                            ),
                    ),
            )
            .child(kit::hint(
                "聊天记录、收到的文件和服务器访问密钥都保存在此目录",
                false,
                cx,
            ))
            .when(self.blocked, |this| {
                this.child(
                    h_flex()
                        .px_3()
                        .py(px(10.))
                        .gap_2()
                        .bg(theme.yellow_light)
                        .text_size(px(13.))
                        .child(kit::icon(Ic::TriangleAlert, 16.).text_color(theme.warning))
                        .child("有传输正在进行，请完成后再迁移"),
                )
            });
        let autostart = h_flex()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(theme.muted_foreground)
                    .child("开启后，下次登录系统时 Local Echo 会自动在后台启动"),
            )
            .child(
                Switch::new("autostart")
                    .checked(self.autostart)
                    .on_change(cx.listener(|this, value: &bool, window, cx| {
                        match desktop::set_autostart(*value) {
                            Ok(()) => this.autostart = *value,
                            Err(e) => {
                                window.push_notification(Notification::error(format!("{e:#}")), cx)
                            }
                        }
                        cx.notify();
                    })),
            );
        let about = h_flex()
            .gap_3()
            .child(kit::mark(20.).text_color(theme.primary))
            .child(
                div()
                    .flex_1()
                    .text_size(px(14.))
                    .font_semibold()
                    .child("Local Echo"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child(format!("版本 {}", env!("CARGO_PKG_VERSION"))),
            );
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(56.))
                    .flex_none()
                    .px_6()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(px(16.))
                    .font_semibold()
                    .child("设置"),
            )
            .child(
                v_flex()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .gap_6()
                    .max_w(px(640.))
                    .child(self.render_ips(window, cx))
                    .child(section("数据目录", data_dir, cx))
                    .child(section("开机自动启动", autostart, cx))
                    .child(section("关于", about, cx))
                    .child(
                        h_flex()
                            .gap_4()
                            .flex_wrap()
                            .child(
                                Button::new("quit")
                                    .secondary()
                                    .icon(kit::icon(Ic::LogOut, 16.))
                                    .label("退出 Local Echo")
                                    .text_color(theme.danger)
                                    .on_click(|_, _, cx| cx.quit()),
                            )
                            .child(kit::hint(
                                "关闭窗口时 Local Echo 会继续在托盘中运行",
                                false,
                                cx,
                            )),
                    ),
            )
    }
}
