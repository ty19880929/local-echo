//! Windows：把应用图标嵌入 exe（设计 10）。
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.compile().expect("无法嵌入 Windows 图标");
    }
}
