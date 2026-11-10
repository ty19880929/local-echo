//! 与机器交互：SSH / SFTP（设计 5）。

use std::{path::Path, process::Command, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use russh::{
    client::{self, Handle, KeyboardInteractiveAuthResponse},
    keys::{
        Algorithm, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate, ssh_key::LineEnding,
    },
};
use russh_sftp::{
    client::SftpSession,
    protocol::{FileAttributes, OpenFlags},
};
use tokio::{io::AsyncWriteExt, sync::mpsc::UnboundedSender, time::timeout};

use crate::{
    core::{Core, Event},
    data,
    db::{Contact, Message},
    peer,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    /// 不校验主机指纹（已确认，设计 5.2）。
    async fn check_server_key(
        &mut self,
        _key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// 缓存的服务器连接：持有 `Handle` 才能保持 SSH 会话存活。
pub struct HostConn {
    _handle: Handle<Client>,
    pub sftp: SftpSession,
}

/// 密钥登录失败的标记错误，UI 据此显示「重新授权」。
#[derive(Debug)]
pub struct AuthFailed;

impl std::fmt::Display for AuthFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("无法登录该服务器，访问密钥可能已失效")
    }
}

impl std::error::Error for AuthFailed {}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u32>,
}

/// 解析 `host[:port]`，默认端口 22。
pub fn parse_addr(input: &str) -> (String, u16) {
    let input = input.trim();
    if let Some((host, port)) = input.rsplit_once(':')
        && !host.contains(':')
        && let Ok(port) = port.parse()
    {
        return (host.to_string(), port);
    }
    (input.to_string(), 22)
}

async fn connect(host: &str, port: u16) -> Result<Handle<Client>> {
    let config = Arc::new(client::Config::default());
    timeout(
        CONNECT_TIMEOUT,
        client::connect(config, (host, port), Client),
    )
    .await
    .map_err(|_| anyhow!("无法连接 {host}:{port}"))?
    .map_err(|_| anyhow!("无法连接 {host}:{port}"))
}

/// password 认证；失败且服务器提供 keyboard-interactive 时用同一密码应答（设计 5.2）。
async fn auth_password(handle: &mut Handle<Client>, user: &str, password: &str) -> Result<bool> {
    if handle
        .authenticate_password(user, password)
        .await?
        .success()
    {
        return Ok(true);
    }
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

async fn auth_key(handle: &mut Handle<Client>, user: &str, key: PrivateKey) -> Result<bool> {
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    Ok(handle.authenticate_publickey(user, key).await?.success())
}

async fn open_sftp(handle: &Handle<Client>) -> Result<SftpSession> {
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    Ok(SftpSession::new(channel.into_stream()).await?)
}

/// 读取工具密钥，首次使用时生成（设计 5.1）。
fn load_or_create_key(core: &Core) -> Result<PrivateKey> {
    let path = core.key_path();
    if path.exists() {
        return Ok(PrivateKey::from_openssh(std::fs::read(&path)?)?);
    }
    let mut key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    key.set_comment(format!("local-echo@{}", data::hostname()));
    std::fs::write(&path, key.to_openssh(LineEnding::LF)?.as_bytes())?;
    data::restrict_key(&path)?;
    Ok(key)
}

/// 通过 SFTP 把公钥追加到 `~/.ssh/authorized_keys`，不依赖远端 shell（设计 5.2）。
async fn deploy_key(sftp: &SftpSession, public_key: &str) -> Result<()> {
    let public_key = public_key.trim();
    if !sftp.try_exists(".ssh").await? {
        sftp.create_dir(".ssh").await?;
    }
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o700);
    sftp.set_metadata(".ssh", attrs).await?;

    let file = ".ssh/authorized_keys";
    let existing = match sftp.read(file).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    };
    if !existing.lines().any(|line| line.trim() == public_key) {
        let mut f = sftp
            .open_with_flags(
                file,
                OpenFlags::CREATE | OpenFlags::APPEND | OpenFlags::WRITE,
            )
            .await?;
        let line = if existing.is_empty() || existing.ends_with('\n') {
            format!("{public_key}\n")
        } else {
            format!("\n{public_key}\n")
        };
        f.write_all(line.as_bytes()).await?;
        f.shutdown().await?;
    }
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o600);
    sftp.set_metadata(file, attrs).await?;
    Ok(())
}

/// 添加服务器 / 重新授权：密码登录 → 部署公钥 → 密钥复验，返回主目录（设计 5.2）。
/// `step` 依次收到 0..=3：连接、验证密码、部署密钥、验证密钥登录。
pub async fn authorize(
    core: &Core,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    step: UnboundedSender<usize>,
) -> Result<String> {
    let _ = step.send(0);
    let mut handle = connect(host, port).await?;
    let _ = step.send(1);
    if !auth_password(&mut handle, user, password).await? {
        bail!("用户名或密码错误");
    }
    let _ = step.send(2);
    let key = load_or_create_key(core)?;
    let sftp = open_sftp(&handle).await?;
    deploy_key(&sftp, &key.public_key().to_openssh()?)
        .await
        .context("部署访问密钥失败")?;
    let _ = step.send(3);
    let mut handle = connect(host, port).await?;
    if !auth_key(&mut handle, user, key).await? {
        bail!("公钥已写入，但无法用密钥登录：服务器可能禁用了公钥认证");
    }
    let sftp = open_sftp(&handle).await?;
    Ok(sftp.canonicalize(".").await?)
}

/// 取得（或建立）该服务器的缓存连接。
pub async fn session(core: &Core, contact: &Contact) -> Result<Arc<HostConn>> {
    let mut hosts = core.hosts.lock().await;
    if let Some(conn) = hosts.get(&contact.id) {
        return Ok(conn.clone());
    }
    let key = load_or_create_key(core)?;
    let mut handle = connect(&contact.host, contact.port).await?;
    if !auth_key(&mut handle, &contact.user, key).await? {
        core.emit(Event::HostAuthFailed(contact.id));
        return Err(AuthFailed.into());
    }
    let sftp = open_sftp(&handle).await?;
    let conn = Arc::new(HostConn {
        _handle: handle,
        sftp,
    });
    hosts.insert(contact.id, conn.clone());
    Ok(conn)
}

/// 丢弃缓存连接（出错或修改地址后）。
pub async fn forget(core: &Core, contact_id: i64) {
    core.hosts.lock().await.remove(&contact_id);
}

async fn with_session<T>(
    core: &Core,
    contact: &Contact,
    f: impl AsyncFnOnce(&SftpSession) -> Result<T>,
) -> Result<T> {
    let conn = session(core, contact).await?;
    let result = f(&conn.sftp).await;
    if result.is_err() {
        forget(core, contact.id).await;
    }
    result
}

/// 列出远程目录，返回规范化后的路径和条目（目录在前）（设计 5.4）。
pub async fn list_dir(core: &Core, contact: &Contact, path: &str) -> Result<(String, Vec<Entry>)> {
    with_session(core, contact, async |sftp| {
        let dir = sftp.canonicalize(path).await?;
        let mut entries = Vec::new();
        for entry in sftp.read_dir(&dir).await? {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let meta = entry.metadata();
            let is_dir = if meta.is_symlink() {
                let target = join_remote(&dir, &name);
                sftp.metadata(&target)
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
            } else {
                meta.is_dir()
            };
            entries.push(Entry {
                name,
                is_dir,
                size: meta.size.unwrap_or(0),
                mtime: meta.mtime,
            });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok((dir, entries))
    })
    .await
}

pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent_remote(dir: &str) -> String {
    match dir.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((parent, _)) => parent.into(),
    }
}

/// 上传一条已入库的消息（本地原文件 → remote_path）。
pub async fn upload(core: Arc<Core>, contact: Contact, msg: Message) {
    let result = with_session(&core, &contact, async |sftp| {
        let local = core.resolve(msg.path.as_deref().unwrap_or_default());
        let mut file = tokio::fs::File::open(&local)
            .await
            .with_context(|| format!("无法读取 {}", local.display()))?;
        let mut remote = sftp
            .create(msg.remote_path.clone().unwrap_or_default())
            .await?;
        peer::copy_progress(
            &core,
            &msg.id,
            &mut file,
            &mut remote,
            msg.size.unwrap_or(0),
        )
        .await?;
        remote.shutdown().await?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("上传失败：{e:#}"));
    core.finish(&msg.id, result);
}

/// 下载一条已入库的消息（remote_path → 本地 files/）。
pub async fn download(core: Arc<Core>, contact: Contact, msg: Message) {
    let local = core.resolve(msg.path.as_deref().unwrap_or_default());
    let result = with_session(&core, &contact, async |sftp| {
        let mut remote = sftp
            .open(msg.remote_path.clone().unwrap_or_default())
            .await?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&local)
            .await?;
        peer::copy_progress(
            &core,
            &msg.id,
            &mut remote,
            &mut file,
            msg.size.unwrap_or(0),
        )
        .await?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("下载失败：{e:#}"));
    if result.is_err() {
        let _ = std::fs::remove_file(&local);
    }
    core.finish(&msg.id, result);
}

// ---------------------------------------------------------------- 在终端中打开（设计 5.6）

fn ssh_args(key: &Path, contact: &Contact) -> Vec<String> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let dir = contact.remote_dir.replace('\'', r"'\''");
    vec![
        "-i".into(),
        key.to_string_lossy().into_owned(),
        "-p".into(),
        contact.port.to_string(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        format!("UserKnownHostsFile={null}"),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-t".into(),
        format!("{}@{}", contact.user, contact.host),
        format!("cd '{dir}'; exec $SHELL -l"),
    ]
}

pub fn open_terminal(core: &Core, contact: &Contact) -> Result<()> {
    let key = core.key_path();
    if !key.exists() {
        return Err(AuthFailed.into());
    }
    let args = ssh_args(&key, contact);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        Command::new("ssh")
            .args(&args)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let (program, prefix) = linux_terminal().ok_or_else(|| anyhow!("未找到可用的终端程序"))?;
        Command::new(program)
            .args(prefix)
            .arg("ssh")
            .args(&args)
            .spawn()?;
        Ok(())
    }
}

/// 终端名 → 拉起它所用的程序和参数前缀。
#[cfg(not(windows))]
fn terminal_command(name: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match name {
        n if n.starts_with("ptyxis") => ("ptyxis", &["--new-window", "--"]),
        n if n.starts_with("gnome-terminal") => ("gnome-terminal", &["--"]),
        n if n.starts_with("konsole") => ("konsole", &["-e"]),
        n if n.starts_with("xfce4-terminal") => ("xfce4-terminal", &["-x"]),
        n if n.starts_with("kitty") => ("kitty", &[]),
        n if n.starts_with("foot") => ("foot", &[]),
        n if n.starts_with("alacritty") => ("alacritty", &["-e"]),
        n if n.starts_with("wezterm") => ("wezterm", &["start", "--"]),
        "xterm" | "uxterm" | "lxterm" => ("xterm", &["-e"]),
        _ => return None,
    })
}

#[cfg(not(windows))]
fn linux_terminal() -> Option<(String, Vec<&'static str>)> {
    if let Some(path) = which("x-terminal-emulator") {
        // 解析软链接得到真实终端；不认识时按 Debian 规范使用 `-e`。
        let real = std::fs::canonicalize(&path).ok();
        let name = real
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        return Some(match name.as_deref().and_then(terminal_command) {
            Some((program, prefix)) if which(program).is_some() => {
                (program.to_string(), prefix.to_vec())
            }
            _ => (path.to_string_lossy().into_owned(), vec!["-e"]),
        });
    }
    [
        "ptyxis",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "kitty",
        "foot",
        "alacritty",
        "wezterm",
        "xterm",
    ]
    .into_iter()
    .filter_map(terminal_command)
    .find(|(program, _)| which(program).is_some())
    .map(|(program, prefix)| (program.to_string(), prefix.to_vec()))
}

#[cfg(not(windows))]
fn which(program: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|dir| Path::new(dir).join(program))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_addr("10.0.0.5"), ("10.0.0.5".into(), 22));
        assert_eq!(parse_addr(" 10.0.0.5:2222 "), ("10.0.0.5".into(), 2222));
    }

    #[test]
    fn remote_paths() {
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/home/a", "b"), "/home/a/b");
        assert_eq!(parent_remote("/home/a"), "/home");
        assert_eq!(parent_remote("/home"), "/");
        assert_eq!(parent_remote("/"), "/");
    }
}

/// 需要一台可登录的 SSH 服务器：
/// `LE_SSH=host:port LE_SSH_USER=u LE_SSH_PASS=p cargo test host -- --ignored`
#[cfg(test)]
mod ssh_tests {
    use super::*;
    use crate::db::{ContactKind, Db, MsgKind, Status, now_ms};
    use tokio::sync::mpsc::unbounded_channel;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn authorize_list_upload_download() {
        let (addr, user, pass) = (
            std::env::var("LE_SSH").unwrap(),
            std::env::var("LE_SSH_USER").unwrap(),
            std::env::var("LE_SSH_PASS").unwrap(),
        );
        let (host, port) = parse_addr(&addr);
        let root = std::env::temp_dir().join(format!("le-host-{}", uuid::Uuid::new_v4()));
        data::ensure_layout(&root).unwrap();
        let (tx, mut rx) = unbounded_channel();
        let core = Core::new(
            root.clone(),
            Db::open(&root.join(data::DB_FILE)).unwrap(),
            tokio::runtime::Handle::current(),
            tx,
        )
        .unwrap();

        let (step_tx, mut step_rx) = unbounded_channel();
        assert!(
            authorize(&core, &host, port, &user, "wrong", step_tx.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("用户名或密码错误")
        );
        let home = authorize(&core, &host, port, &user, &pass, step_tx)
            .await
            .unwrap();
        let mut steps = vec![];
        while let Ok(s) = step_rx.try_recv() {
            steps.push(s);
        }
        assert_eq!(&steps[steps.len() - 4..], &[0, 1, 2, 3]);

        let mut contact = Contact {
            id: 0,
            kind: ContactKind::Host,
            name: "测试机".into(),
            host,
            port,
            user,
            peer_id: None,
            remote_dir: home.clone(),
        };
        contact.id = core.db(|db| db.insert_contact(&contact)).unwrap();

        let (dir, entries) = list_dir(&core, &contact, &home).await.unwrap();
        assert_eq!(dir, home);
        assert!(entries.iter().any(|e| e.name == ".ssh" && e.is_dir));

        let src = root.join("up.bin");
        let bytes: Vec<u8> = (0..5_000_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(&src, &bytes).unwrap();
        let remote = join_remote(&home, "le-test-up.bin");
        let up = Message {
            id: "up".into(),
            contact_id: contact.id,
            outgoing: true,
            kind: MsgKind::File,
            body: "up.bin".into(),
            path: Some(core.store(&src)),
            remote_path: Some(remote.clone()),
            size: Some(bytes.len() as u64),
            status: Status::Pending,
            ts: now_ms(),
        };
        core.db(|db| db.upsert_message(&up)).unwrap();
        upload(core.clone(), contact.clone(), up.clone()).await;

        let (local, _) = data::create_unique(&core.files_dir(), "le-test-up.bin").unwrap();
        let down = Message {
            id: "down".into(),
            outgoing: false,
            path: Some(core.store(&local)),
            ..up.clone()
        };
        core.db(|db| db.upsert_message(&down)).unwrap();
        download(core.clone(), contact.clone(), down).await;
        assert_eq!(std::fs::read(&local).unwrap(), bytes);

        let mut statuses = vec![];
        while let Ok(e) = rx.try_recv() {
            if let Event::Status { id, status, error } = e {
                statuses.push((id, status, error));
            }
        }
        assert!(
            statuses.iter().all(|(_, s, _)| *s == Status::Ok),
            "{statuses:?}"
        );

        // 已部署密钥后，系统 ssh 也能免密登录（「在终端中打开」所用的同一套参数）
        let mut args = ssh_args(&core.key_path(), &contact);
        args.retain(|a| a != "-t");
        *args.last_mut().unwrap() = format!("cd '{}' && pwd", contact.remote_dir);
        let out = Command::new("ssh")
            .args(["-o", "BatchMode=yes"])
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), home);

        let conn = session(&core, &contact).await.unwrap();
        conn.sftp.remove_file(remote).await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
