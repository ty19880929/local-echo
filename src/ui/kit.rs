//! 设计规格（docs/ui，Claude Design 交付）中反复出现的视觉单元。
//! 颜色一律取自主题 token（assets/theme.json），尺寸按规格页数值。

use gpui_kit::{
    assets::IconName as Ic,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::Dialog,
        h_flex, v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

/// 底栏高度 token（v0.2 U3）：不含 1 px 上边框。侧边栏「设置」区、联系人输入区（单行时）、
/// 服务器底栏都用它，三处上边框落在同一像素行。
pub const FOOTER_H: f32 = 60.;

/// 带 1 px 上边框的底栏：gpui 的尺寸含边框，所以总高是 `FOOTER_H + 1`。
pub fn footer_height() -> Pixels {
    px(FOOTER_H + 1.)
}

/// 表头（远程目录浏览器、本机 IP 共用）：高 32，table.head 底，12/600。左右内边距由调用方给出。
pub fn table_head(cx: &App) -> Div {
    h_flex()
        .h(px(32.))
        .flex_none()
        .gap_3()
        .bg(cx.theme().table_head)
        .text_size(px(12.))
        .font_semibold()
        .text_color(cx.theme().table_head_foreground)
}

/// 表格行：高 36，table.row.border 行线，14 px。行线在上还是在下、左右内边距由调用方给出。
pub fn table_row(cx: &App) -> Div {
    h_flex()
        .h(px(36.))
        .flex_none()
        .gap_3()
        .border_color(cx.theme().table_row_border)
        .text_size(px(14.))
}

/// 品牌图形（两点 + 相向弧线），按前景色着色。
pub fn mark(size: f32) -> Svg {
    let path = if size <= 16. {
        "brand/mark-16.svg"
    } else {
        "brand/mark.svg"
    };
    svg().path(path).size(px(size)).flex_none()
}

pub fn icon(name: Ic, size: f32) -> Icon {
    Icon::new(name).size(px(size))
}

pub fn mono(cx: &App) -> SharedString {
    cx.theme().mono_font_family.clone()
}

/// 32 × 32 方形头像：联系人取昵称首字。
pub fn avatar_text(name: &str, cx: &App) -> Div {
    let initial: String = name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().secondary_hover)
        .text_size(px(14.))
        .font_semibold()
        .child(initial)
}

/// 32 × 32 方形头像：服务器图标。
pub fn avatar_server(cx: &App) -> Div {
    div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().secondary_hover)
        .child(icon(Ic::Server, 16.))
}

/// 未读徽标：高 20，最小宽 20，primary 底。
pub fn badge(count: u32, cx: &App) -> Div {
    div()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().primary)
        .text_color(cx.theme().primary_foreground)
        .text_size(px(12.))
        .font_semibold()
        .child(count.to_string())
}

/// 文件类型图标映射（规格：file-text · file-archive · file-image · file-code · presentation · file）。
pub fn file_icon(name: &str) -> Ic {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or_default();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tar.xz") {
        return Ic::FileArchive;
    }
    match ext {
        "ppt" | "pptx" | "key" | "odp" => Ic::Presentation,
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" | "xz" | "bz2" | "zst" => Ic::FileArchive,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "fig" | "psd" | "sketch" => {
            Ic::FileImage
        }
        "rs" | "js" | "ts" | "py" | "go" | "c" | "h" | "cpp" | "java" | "sh" | "json" | "yaml"
        | "yml" | "toml" | "xml" | "html" | "css" | "conf" | "ini" => Ic::FileCode,
        "txt" | "md" | "log" | "csv" | "pdf" | "doc" | "docx" => Ic::FileText,
        _ => Ic::File,
    }
}

/// 单行中间省略：按字宽估算（CJK 1em，其余约 0.56em），超出 `max_em` 时保留首尾。
pub fn middle_ellipsis(text: &str, max_em: f32) -> String {
    let width = |c: char| if c.is_ascii() { 0.56 } else { 1.0 };
    let total: f32 = text.chars().map(width).sum();
    if total <= max_em {
        return text.to_string();
    }
    let budget = (max_em - 1.0) / 2.0;
    let chars: Vec<char> = text.chars().collect();
    let (mut head, mut used) = (String::new(), 0.0);
    for &c in &chars {
        if used + width(c) > budget {
            break;
        }
        used += width(c);
        head.push(c);
    }
    let (mut tail, mut used) = (Vec::new(), 0.0);
    for &c in chars.iter().rev() {
        if used + width(c) > budget {
            break;
        }
        used += width(c);
        tail.push(c);
    }
    tail.reverse();
    format!("{head}…{}", tail.into_iter().collect::<String>())
}

/// 日期分隔文案：今天 / 昨天 11月8日 / 11月6日 / 2025年11月6日。
pub fn date_label(ts: i64) -> String {
    use chrono::{Datelike as _, Local, TimeZone as _};
    let Some(time) = Local.timestamp_millis_opt(ts).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let date = time.date_naive();
    if date == today {
        "今天".into()
    } else if Some(date) == today.pred_opt() {
        format!("昨天 {}月{}日", date.month(), date.day())
    } else if date.year() == today.year() {
        format!("{}月{}日", date.month(), date.day())
    } else {
        format!("{}年{}月{}日", date.year(), date.month(), date.day())
    }
}

pub fn same_day(a: i64, b: i64) -> bool {
    use chrono::{Local, TimeZone as _};
    let day = |ts| {
        Local
            .timestamp_millis_opt(ts)
            .single()
            .map(|t| t.date_naive())
    };
    day(a) == day(b)
}

/// 日期分隔：12 px muted，两侧 1 px 线。
pub fn date_separator(label: String, cx: &App) -> Div {
    let line = || div().flex_1().h(px(1.)).bg(cx.theme().border);
    h_flex()
        .w_full()
        .gap_3()
        .text_size(px(12.))
        .text_color(cx.theme().muted_foreground)
        .child(line())
        .child(label)
        .child(line())
}

/// 进度条：轨道 primary 15%，填充 progress_bar。
pub fn progress_bar(percent: f32, height: f32, cx: &App) -> Div {
    div()
        .w_full()
        .h(px(height))
        .bg(cx.theme().primary.opacity(0.15))
        .child(
            div()
                .h_full()
                .w(relative(percent.clamp(0., 100.) / 100.))
                .bg(cx.theme().progress_bar),
        )
}

/// 说明框：muted 底，1 px 边框，内边距 12，13/20。
pub fn notice(icon_name: Ic, icon_color: Hsla, text: &str, cx: &App) -> Div {
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .p_3()
        .bg(cx.theme().muted)
        .border_1()
        .border_color(cx.theme().border)
        .text_size(px(13.))
        .line_height(px(20.))
        .child(
            div()
                .mt(px(2.))
                .text_color(icon_color)
                .child(icon(icon_name, 16.)),
        )
        .child(div().flex_1().min_w_0().child(text.to_string()))
}

/// 对话框外壳：去掉组件自带的内边距与关闭按钮，由内容自绘标题区、内容区和按钮区（规格：20 24 0 / 16 24 / 8 24 20）。
pub fn dialog_shell(dialog: Dialog, width: f32, cx: &App) -> Dialog {
    dialog
        .w(px(width))
        .p_0()
        .close_button(false)
        .bg(cx.theme().popover)
}

/// 对话框标题区：16/600 + 关闭 x。
pub fn dialog_header(title: impl Into<SharedString>, cx: &App) -> Div {
    h_flex()
        .px_6()
        .pt(px(20.))
        .items_center()
        .child(
            div()
                .flex_1()
                .text_size(px(16.))
                .line_height(px(22.))
                .font_semibold()
                .child(title.into()),
        )
        .child(
            Button::new("dialog-x")
                .ghost()
                .xsmall()
                .icon(icon(Ic::X, 16.))
                .text_color(cx.theme().muted_foreground)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
}

/// 对话框按钮区：右对齐，间距 8。
pub fn dialog_footer() -> Div {
    h_flex().px_6().pt_2().pb(px(20.)).justify_end().gap_2()
}

pub fn cancel_button() -> Button {
    Button::new("dialog-cancel")
        .secondary()
        .label("取消")
        .on_click(|_, window, cx| window.close_dialog(cx))
}

/// 表单字段：标签 13/600，标签与输入 8。
pub fn field(label: &str, input: impl IntoElement) -> Div {
    v_flex()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .font_semibold()
                .child(label.to_string()),
        )
        .child(input)
}

/// 只读字段：muted 底 + lock 14，文字 muted。
pub fn locked_field(label: &str, value: &str, cx: &App) -> Div {
    field(
        label,
        h_flex()
            .h(px(32.))
            .px_3()
            .gap_2()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .text_size(px(14.))
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_1().child(value.to_string()))
            .child(icon(Ic::Lock, 14.)),
    )
}

/// 12 px 提示或错误。
pub fn hint(text: impl Into<SharedString>, error: bool, cx: &App) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(if error {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        })
        .child(text.into())
}

/// 链接样式文字（「重试」「更改」「编辑」）。
pub fn link_text(text: &str, cx: &App) -> Div {
    div()
        .text_color(cx.theme().link)
        .font_semibold()
        .cursor_pointer()
        .child(text.to_string())
}

/// 确认类对话框（420 宽）：删除、迁移等（设计 01-D、09-C、09-D）。
pub struct Confirm {
    pub title: String,
    pub path: Option<String>,
    pub body: String,
    pub ok: &'static str,
    pub danger: bool,
}

pub fn open_confirm(
    confirm: Confirm,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_ok = std::rc::Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, cx| {
        let (enter, click) = (on_ok.clone(), on_ok.clone());
        let ok = Button::new("dialog-ok")
            .label(confirm.ok)
            .map(|b| {
                if confirm.danger {
                    b.danger()
                } else {
                    b.primary()
                }
            })
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                click(window, cx);
            });
        dialog_shell(dialog, 420., cx)
            .on_ok(move |_, window, cx| {
                enter(window, cx);
                true
            })
            .child(
                v_flex()
                    .child(
                        h_flex().px_6().pt(px(20.)).child(
                            div()
                                .flex_1()
                                .text_size(px(16.))
                                .line_height(px(22.))
                                .font_semibold()
                                .child(confirm.title.clone()),
                        ),
                    )
                    .child(
                        v_flex()
                            .px_6()
                            .pt_3()
                            .pb_4()
                            .gap_3()
                            .text_size(px(14.))
                            .line_height(px(22.))
                            .when_some(confirm.path.clone(), |this, path| {
                                this.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .bg(cx.theme().muted)
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .font_family(mono(cx))
                                        .text_size(px(13.))
                                        .child(path),
                                )
                            })
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(confirm.body.clone()),
                            ),
                    )
                    .child(dialog_footer().child(cancel_button()).child(ok)),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::{Ic, file_icon, middle_ellipsis};

    #[test]
    fn ellipsis() {
        assert_eq!(middle_ellipsis("short.txt", 20.), "short.txt");
        let s = middle_ellipsis("季度汇报-终版-v3-修订-2026-11.pptx", 16.);
        assert!(
            s.contains('…') && s.starts_with("季度") && s.ends_with(".pptx"),
            "{s}"
        );
    }

    #[test]
    fn icons() {
        assert_eq!(file_icon("a.PPTX"), Ic::Presentation);
        assert_eq!(file_icon("logs.tar.gz"), Ic::FileArchive);
        assert_eq!(file_icon("nginx.conf"), Ic::FileCode);
        assert_eq!(file_icon("app.log"), Ic::FileText);
        assert_eq!(file_icon("setup.msi"), Ic::File);
    }
}
