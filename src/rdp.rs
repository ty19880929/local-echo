//! 在远程桌面打开（v0.2 F1，设计 5.7）：先探测服务器的 RDP 端口，再拉起本机的远程桌面客户端。
//! 用户名预填该服务器的 SSH 用户名；密码不保存，由客户端自己询问（与设计 5.1「密码不落盘」一致）。

use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::{net::TcpStream, time::timeout};

use crate::db::Contact;

/// 端口探测的超时，与联系人连接的超时一致。
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Linux 上找不到任何支持 RDP 的客户端；界面据此弹出「未找到远程桌面客户端」对话框。
#[derive(Debug)]
pub struct NoClient;

impl std::fmt::Display for NoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("未找到远程桌面客户端")
    }
}

impl std::error::Error for NoClient {}

/// 去掉 IPv6 地址可能带的方括号。
fn bare_host(host: &str) -> &str {
    host.trim()
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host.trim())
}

/// 用于 `host:port` 拼接：IPv6 地址加方括号。
fn bracket_host(host: &str) -> String {
    let bare = bare_host(host);
    if bare.contains(':') {
        format!("[{bare}]")
    } else {
        bare.to_string()
    }
}

/// 「编辑服务器」的远程桌面端口：留空按 3389 处理，范围 1–65535，超出返回 `None`。
pub fn parse_port(input: &str) -> Option<u16> {
    match input.trim() {
        "" => Some(crate::db::DEFAULT_RDP_PORT),
        text => text.parse::<u16>().ok().filter(|p| *p != 0),
    }
}

/// TCP 探测 `host:port`，最多 3 秒。客户端拉起后的失败提示五花八门，先在这里把
/// 「服务器没开远程桌面」这种最常见的错误用中文说清楚。
pub async fn probe(host: &str, port: u16) -> Result<()> {
    match timeout(PROBE_TIMEOUT, TcpStream::connect((bare_host(host), port))).await {
        Ok(Ok(_)) => Ok(()),
        _ => Err(anyhow!(
            "无法连接 {host}:{port}：服务器未开启远程桌面，或端口不可达。可在「编辑服务器」中修改远程桌面端口"
        )),
    }
}

/// `rdp://<user>@<host>:<port>`。用户名含 `@ : / ? # %` 时省略用户名部分，由客户端询问，
/// 避免各客户端解码方式不同导致登录用户错误。
#[cfg_attr(windows, allow(dead_code))]
pub fn rdp_uri(host: &str, port: u16, user: Option<&str>) -> String {
    let host = bracket_host(host);
    match user.filter(|u| !u.is_empty() && !u.contains(['@', ':', '/', '?', '#', '%'])) {
        Some(user) => format!("rdp://{user}@{host}:{port}"),
        None => format!("rdp://{host}:{port}"),
    }
}

/// 拉起本机的远程桌面客户端。会运行探测命令，需在后台线程调用。
pub fn launch(contact: &Contact) -> Result<()> {
    platform::launch(contact)
}

// ---------------------------------------------------------------- Windows：mstsc

/// mstsc 的 `.rdp` 文件：与 mstsc 自己保存的格式一致（UTF-16LE 带 BOM，CRLF），
/// 用户名里有中文也不会乱码。只写地址和用户名，其余设置用 mstsc 的默认值，不含密码。
#[cfg(any(windows, test))]
pub fn rdp_file(host: &str, port: u16, user: &str) -> Vec<u8> {
    let mut text = format!("full address:s:{}:{port}\r\n", bracket_host(host));
    if !user.is_empty() {
        text.push_str(&format!("username:s:{user}\r\n"));
    }
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

#[cfg(windows)]
mod platform {
    use std::{path::PathBuf, process::Command};

    use anyhow::Result;

    use super::rdp_file;
    use crate::{db::Contact, desktop};

    /// 不用 `mstsc /v:`（不能预填用户名），改为每次覆盖写入
    /// `%TEMP%\local-echo\rdp-<id>.rdp` 再交给 mstsc 打开。
    pub fn launch(contact: &Contact) -> Result<()> {
        let dir = std::env::temp_dir().join("local-echo");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("rdp-{}.rdp", contact.id));
        std::fs::write(
            &file,
            rdp_file(&contact.host, contact.rdp_port, &contact.user),
        )?;
        let mstsc = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("mstsc.exe"))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| "mstsc.exe".into());
        desktop::reap(Command::new(mstsc).arg(&file).spawn()?);
        Ok(())
    }
}

// ---------------------------------------------------------------- Linux：按顺序探测客户端

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
enum Client {
    Remmina,
    RemminaFlatpak,
    Krdc,
    SdlFreerdp3,
    SdlFreerdp,
    /// GNOME Connections：从命令行接受一个 `rdp://` 参数并直接连接（2026-11-11 实测，R14）。
    GnomeConnections,
    /// 系统默认的 `rdp://` 处理程序（`xdg-mime query default x-scheme-handler/rdp`）。
    Default,
}

/// 探测顺序（设计 5.7）。Remmina 注册的 `rdp://` 处理程序执行的是不带 `-c` 的
/// `remmina rdp://…`，直接调用 `remmina -c` 行为更确定，所以系统默认处理程序只作兜底。
/// GNOME Connections（Fedora Workstation 默认自带）从命令行传入 `rdp://` 能直接连接，
/// 2026-11-11 在 Docker 中实测通过后加入（R14）。
#[cfg_attr(windows, allow(dead_code))]
const LINUX_ORDER: [Client; 7] = [
    Client::Remmina,
    Client::RemminaFlatpak,
    Client::Krdc,
    Client::SdlFreerdp3,
    Client::SdlFreerdp,
    Client::GnomeConnections,
    Client::Default,
];

/// 按探测顺序找出第一个可用的客户端。`which` 判断可执行文件是否在 PATH 中；
/// `flatpak_remmina`、`rdp_handler` 运行 Flatpak / xdg-mime 的探测命令。三者在单元测试中替换。
#[cfg_attr(windows, allow(dead_code))]
fn detect(
    which: &dyn Fn(&str) -> bool,
    flatpak_remmina: &dyn Fn() -> bool,
    rdp_handler: &dyn Fn() -> bool,
) -> Option<Client> {
    LINUX_ORDER.into_iter().find(|client| match client {
        Client::Remmina => which("remmina"),
        // 通过 Flathub 安装时不在 PATH 中，需要单独探测。
        Client::RemminaFlatpak => which("flatpak") && flatpak_remmina(),
        Client::Krdc => which("krdc"),
        Client::SdlFreerdp3 => which("sdl-freerdp3"),
        Client::SdlFreerdp => which("sdl-freerdp"),
        Client::GnomeConnections => which("gnome-connections"),
        Client::Default => which("xdg-open") && rdp_handler(),
    })
}

/// 客户端的拉起命令：(程序, 参数)。
#[cfg_attr(windows, allow(dead_code))]
fn command(client: Client, contact: &Contact) -> (&'static str, Vec<String>) {
    let (host, port, user) = (&contact.host, contact.rdp_port, contact.user.as_str());
    let uri = || rdp_uri(host, port, Some(user));
    match client {
        Client::Remmina => ("remmina", vec!["-c".into(), uri()]),
        Client::RemminaFlatpak => (
            "flatpak",
            vec![
                "run".into(),
                "org.remmina.Remmina".into(),
                "-c".into(),
                uri(),
            ],
        ),
        Client::Krdc => ("krdc", vec![uri()]),
        Client::SdlFreerdp3 | Client::SdlFreerdp => {
            let program = if client == Client::SdlFreerdp3 {
                "sdl-freerdp3"
            } else {
                "sdl-freerdp"
            };
            let mut args = vec![format!("/v:{}:{port}", bracket_host(host))];
            if !user.is_empty() {
                args.push(format!("/u:{user}"));
            }
            args.push(format!("/t:{}", contact.name));
            args.push("/dynamic-resolution".into());
            args.push("+clipboard".into());
            (program, args)
        }
        // 只接受一个参数；按方案不带用户名，由它的认证框询问。
        Client::GnomeConnections => ("gnome-connections", vec![rdp_uri(host, port, None)]),
        Client::Default => ("xdg-open", vec![uri()]),
    }
}

#[cfg(not(windows))]
mod platform {
    use std::process::{Command, Stdio};

    use anyhow::Result;

    use super::{Client, NoClient, command, detect};
    use crate::{db::Contact, desktop, host::which};

    fn succeeds(program: &str, args: &[&str]) -> bool {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn has_rdp_handler() -> bool {
        Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/rdp"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
    }

    pub fn launch(contact: &Contact) -> Result<()> {
        let client: Client = detect(
            &|program| which(program).is_some(),
            &|| succeeds("flatpak", &["info", "org.remmina.Remmina"]),
            &has_rdp_handler,
        )
        .ok_or(NoClient)?;
        let (program, args) = command(client, contact);
        let child = Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .spawn()?;
        desktop::reap(child);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ContactKind, DEFAULT_RDP_PORT};

    fn server(host: &str, user: &str) -> Contact {
        Contact {
            id: 7,
            kind: ContactKind::Host,
            name: "生产服务器".into(),
            host: host.into(),
            port: 22,
            user: user.into(),
            peer_id: None,
            remote_dir: "/root".into(),
            rdp_port: DEFAULT_RDP_PORT,
        }
    }

    #[test]
    fn ports() {
        assert_eq!(parse_port(""), Some(3389));
        assert_eq!(parse_port("  "), Some(3389));
        assert_eq!(parse_port("13389"), Some(13389));
        assert_eq!(parse_port("1"), Some(1));
        assert_eq!(parse_port("65535"), Some(65535));
        for bad in ["0", "65536", "70000", "abc", "-1", "33 89"] {
            assert_eq!(parse_port(bad), None, "{bad}");
        }
    }

    #[test]
    fn uris() {
        assert_eq!(
            rdp_uri("10.0.0.5", 3389, Some("root")),
            "rdp://root@10.0.0.5:3389"
        );
        assert_eq!(rdp_uri("srv.lan", 13389, None), "rdp://srv.lan:13389");
        assert_eq!(rdp_uri("srv.lan", 3389, Some("")), "rdp://srv.lan:3389");
        assert_eq!(
            rdp_uri("fe80::1", 3389, Some("u")),
            "rdp://u@[fe80::1]:3389"
        );
        assert_eq!(rdp_uri("[::1]", 3389, None), "rdp://[::1]:3389");
        assert_eq!(
            rdp_uri("10.0.0.5", 3389, Some("张三")),
            "rdp://张三@10.0.0.5:3389"
        );
        for user in ["a@corp", "dom:u", "a/b", "a?b", "a#b", "a%20b"] {
            assert_eq!(
                rdp_uri("10.0.0.5", 3389, Some(user)),
                "rdp://10.0.0.5:3389",
                "{user}"
            );
        }
    }

    #[test]
    fn rdp_file_is_utf16le_with_bom() {
        let bytes = rdp_file("10.0.0.5", 3389, "张三");
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "full address:s:10.0.0.5:3389\r\nusername:s:张三\r\n"
        );
        let v6 = rdp_file("fe80::1", 3390, "");
        let units: Vec<u16> = v6[2..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "full address:s:[fe80::1]:3390\r\n"
        );
        assert!(!String::from_utf16_lossy(&units).contains("password"));
    }

    #[test]
    fn linux_detection_order() {
        let pick = |installed: &[&str], flatpak: bool, handler: bool| {
            let installed: Vec<String> = installed.iter().map(|s| s.to_string()).collect();
            detect(&|p| installed.iter().any(|i| i == p), &|| flatpak, &|| {
                handler
            })
        };
        let all = [
            "remmina",
            "flatpak",
            "krdc",
            "sdl-freerdp3",
            "sdl-freerdp",
            "gnome-connections",
            "xdg-open",
        ];
        assert_eq!(pick(&all, true, true), Some(Client::Remmina));
        assert_eq!(pick(&all[1..], true, true), Some(Client::RemminaFlatpak));
        // 装了 flatpak 但没有 Remmina 的 Flatpak 版
        assert_eq!(pick(&all[1..], false, true), Some(Client::Krdc));
        assert_eq!(pick(&all[3..], false, true), Some(Client::SdlFreerdp3));
        assert_eq!(pick(&all[4..], false, true), Some(Client::SdlFreerdp));
        assert_eq!(pick(&all[5..], false, true), Some(Client::GnomeConnections));
        assert_eq!(pick(&["xdg-open"], false, true), Some(Client::Default));
        // 有 xdg-open 但没有注册 rdp:// 处理程序
        assert_eq!(pick(&["xdg-open"], false, false), None);
        // 只有终端版 FreeRDP 不算（无法输入密码）
        assert_eq!(pick(&["xfreerdp3", "xfreerdp"], false, false), None);
    }

    #[test]
    fn linux_commands() {
        let c = server("10.0.0.5", "root");
        assert_eq!(
            command(Client::Remmina, &c),
            (
                "remmina",
                vec!["-c".into(), "rdp://root@10.0.0.5:3389".into()]
            )
        );
        assert_eq!(
            command(Client::RemminaFlatpak, &c).1,
            [
                "run",
                "org.remmina.Remmina",
                "-c",
                "rdp://root@10.0.0.5:3389"
            ]
        );
        assert_eq!(
            command(Client::Krdc, &c),
            ("krdc", vec!["rdp://root@10.0.0.5:3389".into()])
        );
        assert_eq!(
            command(Client::SdlFreerdp3, &c),
            (
                "sdl-freerdp3",
                vec![
                    "/v:10.0.0.5:3389".into(),
                    "/u:root".into(),
                    "/t:生产服务器".into(),
                    "/dynamic-resolution".into(),
                    "+clipboard".into()
                ]
            )
        );
        let mut v6 = server("fe80::1", "a@corp");
        v6.rdp_port = 13389;
        assert_eq!(command(Client::SdlFreerdp, &v6).0, "sdl-freerdp");
        assert_eq!(command(Client::SdlFreerdp, &v6).1[0], "/v:[fe80::1]:13389");
        assert_eq!(
            command(Client::GnomeConnections, &c),
            ("gnome-connections", vec!["rdp://10.0.0.5:3389".into()])
        );
        assert_eq!(
            command(Client::Default, &v6),
            ("xdg-open", vec!["rdp://[fe80::1]:13389".into()])
        );
    }

    #[tokio::test]
    async fn probe_reports_closed_port() {
        // 先占一个端口再释放，得到一个大概率没人监听的端口。
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = probe("127.0.0.1", port).await.unwrap_err().to_string();
        assert!(
            err.starts_with(&format!("无法连接 127.0.0.1:{port}：")),
            "{err}"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        probe("127.0.0.1", port).await.unwrap();
    }
}
