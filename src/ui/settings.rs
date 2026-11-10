//! 设置页：数据目录与迁移、开机自动启动、退出、版本（设计 7、8.4）。

use std::path::PathBuf;

use gpui_kit::{
    component::{
        ActiveTheme as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariant, ButtonVariants as _},
        h_flex,
        notification::Notification,
        progress::Progress,
        switch::Switch,
        v_flex,
    },
    *,
};

use super::{core, format_size};
use crate::{data, desktop};

pub struct SettingsView {
    autostart: bool,
    /// 迁移进度（已移动字节, 总字节）。
    migrating: Option<(u64, u64)>,
}

impl SettingsView {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            autostart: desktop::autostart_enabled(),
            migrating: None,
        }
    }

    fn change_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let core = core(cx);
        if target == core.root {
            return;
        }
        if data::has_data(&target) {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let target = target.clone();
                alert
                    .title("使用该目录中的已有数据？")
                    .description("当前数据将保留在原位置。")
                    .show_cancel(true)
                    .cancel_text("取消")
                    .ok_text("使用")
                    .on_ok(move |_, window, cx| {
                        match data::set_root(&target) {
                            Ok(()) => cx.restart(),
                            Err(e) => {
                                window.push_notification(Notification::error(format!("{e:#}")), cx)
                            }
                        }
                        true
                    })
            });
            return;
        }
        if core.db(|db| db.has_pending()).unwrap_or(false) {
            window.push_notification("有传输正在进行，请完成后再迁移", cx);
            return;
        }
        let view = cx.entity().downgrade();
        let description = format!("将全部数据移动到 {}？", target.display());
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, target) = (view.clone(), target.clone());
            alert
                .title("迁移数据目录")
                .description(description.clone())
                .show_cancel(true)
                .cancel_text("取消")
                .ok_text("移动")
                .on_ok(move |_, window, cx| {
                    let target = target.clone();
                    let _ = view.update(cx, |this, cx| this.migrate(target, window, cx));
                    true
                })
        });
    }

    /// 关闭数据库 → 移动数据 → 改写指针文件 → 重启（设计 7.2）。
    fn migrate(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let core = core(cx);
        core.close_db();
        self.migrating = Some((0, 0));
        cx.notify();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let from = core.root.clone();
        let task = core.runtime.spawn_blocking(move || {
            data::migrate(&from, &target, |done, total| {
                let _ = tx.send((done, total));
            })?;
            data::set_root(&target)
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Some(progress) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.migrating = Some(progress);
                    cx.notify();
                });
            }
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => cx.restart(),
                Err(e) => {
                    // 迁移失败：数据可能已部分移动，重启后按指针文件重新打开。
                    this.migrating = None;
                    window.push_notification(Notification::error(format!("迁移失败：{e:#}")), cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn row(label: &str, description: &str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_4()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .min_w_0()
                .child(div().font_medium().child(label.to_string()))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(description.to_string()),
                ),
        )
        .child(control)
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = core(cx).root.clone();
        let data_dir = match self.migrating {
            Some((done, total)) => v_flex()
                .w(px(240.))
                .gap_1()
                .child(Progress::new("migrate").value(if total == 0 {
                    0.
                } else {
                    done as f32 * 100. / total as f32
                }))
                .child(div().text_sm().child(format!(
                    "正在移动 {} / {}",
                    format_size(done),
                    format_size(total)
                )))
                .into_any_element(),
            None => h_flex()
                .gap_2()
                .child(Button::new("open-dir").label("打开").on_click({
                    let root = root.clone();
                    move |_, _, cx| cx.open_with_system(&root)
                }))
                .child(
                    Button::new("change-dir")
                        .label("更改…")
                        .on_click(cx.listener(|this, _, window, cx| this.change_dir(window, cx))),
                )
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .p_6()
            .gap_2()
            .child(div().text_xl().font_semibold().pb_2().child("设置"))
            .child(row(
                "数据目录",
                &format!(
                    "{}\n聊天记录、收到的文件和服务器访问密钥都保存在此目录",
                    root.display()
                ),
                data_dir,
                cx,
            ))
            .child(row(
                "开机自动启动",
                "开启后，下次登录系统时 Local Echo 会自动在后台启动",
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
                cx,
            ))
            .child(row(
                "退出 Local Echo",
                "关闭窗口只会驻留托盘，这里才是彻底退出",
                Button::new("quit")
                    .with_variant(ButtonVariant::Danger)
                    .label("退出")
                    .on_click(|_, _, cx| cx.quit()),
                cx,
            ))
            .child(row(
                "关于",
                &format!("版本 {}", env!("CARGO_PKG_VERSION")),
                div(),
                cx,
            ))
    }
}
