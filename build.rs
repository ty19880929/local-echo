//! Windows 目标：把应用图标嵌入 exe（设计 10）。
//! 按目标平台判断（不是编译机），在 Linux 上交叉编译时 winresource 会调用
//! x86_64-w64-mingw32-windres；CI 在 Windows 上原生构建，行为不变。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/app.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.compile().expect("无法嵌入 Windows 图标");
    }
}
