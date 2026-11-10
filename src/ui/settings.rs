//! 设置页：数据目录与迁移、开机自动启动、关于、退出（设计 7、8.4；视觉见 docs/ui 09）。

use std::path::PathBuf;

use gpui_kit::{
    assets::IconName as Ic,
    component::{
        ActiveTheme as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        notification::Notification,
        switch::Switch,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use super::{core, format_size, kit};
use crate::{data, desktop};

pub struct SettingsView {
    autostart: bool,
    /// 点击「更改…」时有传输正在进行（设计 09-B）。
    blocked: bool,
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
        }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
