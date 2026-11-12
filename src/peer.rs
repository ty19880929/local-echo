//! 与人交互：TCP 短连接协议（设计 4.2）。
//!
//! 请求：`[u32 大端 N][N 字节 JSON 头][size 字节文件内容（仅 image / file）]`
//! 响应：`[u32 大端 M][M 字节 JSON]`

use std::{
    io::{Read as _, Write as _},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use crate::{
    core::{Core, Event},
    data,
    db::{Contact, ContactKind, Message, MsgKind, Status, now_ms},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// 传输中连续 30 秒没有进展判定失败。
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEADER: u32 = 16 << 20;
const CHUNK: usize = 256 * 1024;

#[derive(Serialize, Deserialize, Default, Debug)]
struct Header {
    t: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Reply {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    err: Option<String>,
}

impl Reply {
    fn ok() -> Self {
        Self {
            ok: true,
            ..Default::default()
        }
    }
    fn err(err: impl Into<String>) -> Self {
        Self {
            ok: false,
            err: Some(err.into()),
            ..Default::default()
        }
    }
}

const WRONG_PEER: &str = "wrong_peer";

async fn write_frame<T: Serialize>(w: &mut (impl AsyncWrite + Unpin), value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

async fn read_frame<T: DeserializeOwned>(r: &mut (impl AsyncRead + Unpin)) -> Result<T> {
    let len = r.read_u32().await?;
    if len > MAX_HEADER {
        bail!("消息头过大");
    }
    let mut buf = vec![0; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

/// 复制 `size` 字节并上报进度；连续 30 秒无进展判定失败。与 host 模块共用。
pub async fn copy_progress(
    core: &Core,
    msg_id: &str,
    reader: &mut (impl AsyncRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
    size: u64,
) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut last_report = Instant::now();
    while done < size {
        let want = CHUNK.min((size - done) as usize);
        let n = timeout(IDLE_TIMEOUT, reader.read(&mut buf[..want]))
            .await
            .map_err(|_| anyhow!("传输超时"))??;
        if n == 0 {
            bail!("连接中断");
        }
        timeout(IDLE_TIMEOUT, writer.write_all(&buf[..n]))
            .await
            .map_err(|_| anyhow!("传输超时"))??;
        done += n as u64;
        if last_report.elapsed() >= Duration::from_millis(100) || done == size {
            last_report = Instant::now();
            core.emit(Event::Progress {
                id: msg_id.to_string(),
                done,
            });
        }
    }
    writer.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------- 接收端

/// 监听 47210，处理所有入站请求。
pub async fn serve(core: Arc<Core>, listener: TcpListener) {
    loop {
        let Ok((stream, addr)) = listener.accept().await else {
            continue;
        };
        let core = core.clone();
        tokio::spawn(async move {
            let _ = handle(core, stream, addr).await;
        });
    }
}

async fn handle(core: Arc<Core>, mut stream: TcpStream, addr: SocketAddr) -> Result<()> {
    let header: Header = timeout(IDLE_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| anyhow!("timeout"))??;
    let reply = process(&core, header, &mut stream, addr.ip())
        .await
        .unwrap_or_else(|e| Reply::err(format!("{e:#}")));
    write_frame(&mut stream, &reply).await
}

async fn process(core: &Core, h: Header, stream: &mut TcpStream, ip: IpAddr) -> Result<Reply> {
    if h.t == "show" {
        if ip.is_loopback() {
            core.emit(Event::ShowWindow);
        }
        return Ok(Reply::ok());
    }
    // 防止发错人（设计 4.1）。
    if h.to.as_ref().is_some_and(|to| *to != core.device_id) {
        return Ok(Reply::err(WRONG_PEER));
    }
    let from = h.from.clone().ok_or_else(|| anyhow!("缺少 from"))?;
    let contact = touch_peer(core, &from, h.user.as_deref(), ip)?;

    match h.t.as_str() {
        "hello" => Ok(Reply {
            ok: true,
            id: Some(core.device_id.clone()),
            err: None,
        }),
        "text" => {
            let msg = Message {
                id: h.id.ok_or_else(|| anyhow!("缺少 id"))?,
                contact_id: contact.id,
                outgoing: false,
                kind: MsgKind::Text,
                body: h.text.unwrap_or_default(),
                path: None,
                remote_path: None,
                size: None,
                status: Status::Ok,
                ts: now_ms(),
            };
            core.add_message(&msg)?;
            Ok(Reply::ok())
        }
        "image" | "file" => receive_file(core, h, contact.id, stream)
            .await
            .map(|_| Reply::ok()),
        other => bail!("未知请求 {other}"),
    }
}

/// 按设备 ID 找到联系人：不存在则新建，IP 变了则更新（设计 4.1）。
fn touch_peer(core: &Core, peer_id: &str, user: Option<&str>, ip: IpAddr) -> Result<Contact> {
    let ip = ip.to_canonical().to_string();
    let (contact, changed) = core.db(|db| match db.contact_by_peer(peer_id)? {
        Some(mut c) if c.host != ip => {
            c.host = ip.clone();
            db.update_contact(&c)?;
            Ok((c, true))
        }
        Some(c) => Ok((c, false)),
        None => {
            let mut c = Contact {
                id: 0,
                kind: ContactKind::Peer,
                name: user.unwrap_or(&ip).to_string(),
                host: ip.clone(),
                port: data::PORT,
                user: String::new(),
                peer_id: Some(peer_id.to_string()),
                remote_dir: String::new(),
                rdp_port: crate::db::DEFAULT_RDP_PORT,
            };
            c.id = db.insert_contact(&c)?;
            Ok((c, true))
        }
    })?;
    if changed {
        core.emit(Event::ContactsChanged);
    }
    Ok(contact)
}

async fn receive_file(
    core: &Core,
    h: Header,
    contact_id: i64,
    stream: &mut TcpStream,
) -> Result<()> {
    let id = h.id.ok_or_else(|| anyhow!("缺少 id"))?;
    let size = h.size.ok_or_else(|| anyhow!("缺少 size"))?;
    let original = h.file.unwrap_or_default();
    let (kind, name) = if h.t == "image" {
        (MsgKind::Image, data::image_name(&original))
    } else {
        (MsgKind::File, data::sanitize_file_name(&original))
    };
    let (path, file) = data::create_unique(&core.files_dir(), &name)?;
    let msg = Message {
        id: id.clone(),
        contact_id,
        outgoing: false,
        kind,
        body: file_name(&path),
        path: Some(core.store(&path)),
        remote_path: None,
        size: Some(size),
        status: Status::Pending,
        ts: now_ms(),
    };
    core.add_message(&msg)?;

    let mut file = tokio::fs::File::from_std(file);
    let result = copy_progress(core, &id, stream, &mut file, size).await;
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let failed = result.as_ref().err().map(|e| anyhow!("{e:#}"));
    core.finish(&id, result);
    match failed {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

pub fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- 发送端

async fn connect(ip: &str) -> Result<TcpStream> {
    let addr = format!("{ip}:{}", data::PORT);
    timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow!("连接超时"))?
        .with_context(|| format!("无法连接 {ip}"))
}

/// 添加联系人时握手，返回对方设备 ID（设计 4.3）。
pub async fn hello(core: &Core, ip: &str) -> Result<String> {
    let fail = || anyhow!("无法连接 {ip}：对方未运行 Local Echo，或网络不可达");
    let mut stream = connect(ip).await.map_err(|_| fail())?;
    let header = Header {
        t: "hello".into(),
        from: Some(core.device_id.clone()),
        user: Some(core.user.clone()),
        ..Default::default()
    };
    write_frame(&mut stream, &header)
        .await
        .map_err(|_| fail())?;
    let reply: Reply = timeout(IDLE_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| fail())?
        .map_err(|_| fail())?;
    reply.id.filter(|_| reply.ok).ok_or_else(fail)
}

/// 发送一条已入库的消息，并更新其状态（设计 4.4）。
pub async fn send(core: Arc<Core>, contact: Contact, msg: Message) {
    let result = send_inner(&core, &contact, &msg).await.map_err(|e| {
        if e.to_string() == WRONG_PEER {
            anyhow!(
                "发送失败：{} 当前不是「{}」的设备，请修改 IP",
                contact.host,
                contact.name
            )
        } else {
            anyhow!("发送失败：{e:#}")
        }
    });
    core.finish(&msg.id, result);
}

async fn send_inner(core: &Core, contact: &Contact, msg: &Message) -> Result<()> {
    let mut stream = connect(&contact.host).await?;
    let mut header = Header {
        from: Some(core.device_id.clone()),
        to: contact.peer_id.clone(),
        user: Some(core.user.clone()),
        id: Some(msg.id.clone()),
        ..Default::default()
    };
    let local: Option<PathBuf> = msg.path.as_deref().map(|p| core.resolve(p));
    match msg.kind {
        MsgKind::Text => {
            header.t = "text".into();
            header.text = Some(msg.body.clone());
        }
        MsgKind::Image | MsgKind::File => {
            header.t = if msg.kind == MsgKind::Image {
                "image"
            } else {
                "file"
            }
            .into();
            header.file = Some(msg.body.clone());
            header.size = msg.size;
        }
    }
    write_frame(&mut stream, &header).await?;
    if let Some(path) = local {
        let mut file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("无法读取 {}", path.display()))?;
        copy_progress(core, &msg.id, &mut file, &mut stream, msg.size.unwrap_or(0)).await?;
    }
    let reply: Reply = timeout(IDLE_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| anyhow!("等待对方确认超时"))??;
    match reply.ok {
        true => Ok(()),
        false => Err(anyhow!(reply.err.unwrap_or_default())),
    }
}

/// 第二个实例启动时唤起已运行的实例（设计 8.3）。返回是否成功。
pub fn signal_show() -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], data::PORT).into();
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let body = serde_json::to_vec(&Header {
        t: "show".into(),
        ..Default::default()
    })
    .unwrap();
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    let mut len = [0u8; 4];
    stream.write_all(&frame).is_ok() && stream.read_exact(&mut len).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn instance() -> (Arc<Core>, UnboundedReceiver<Event>, PathBuf) {
        let root = std::env::temp_dir().join(format!("le-peer-{}", uuid::Uuid::new_v4()));
        data::ensure_layout(&root).unwrap();
        let db = Db::open(&root.join(data::DB_FILE)).unwrap();
        let (tx, rx) = unbounded_channel();
        let core = Core::new(root.clone(), db, tokio::runtime::Handle::current(), tx).unwrap();
        (core, rx, root)
    }

    fn outgoing(
        contact: &Contact,
        kind: MsgKind,
        body: &str,
        path: Option<String>,
        size: Option<u64>,
    ) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            contact_id: contact.id,
            outgoing: true,
            kind,
            body: body.into(),
            path,
            remote_path: None,
            size,
            status: Status::Pending,
            ts: now_ms(),
        }
    }

    async fn status_of(rx: &mut UnboundedReceiver<Event>, id: &str) -> (Status, Option<String>) {
        while let Some(event) = rx.recv().await {
            if let Event::Status {
                id: got,
                status,
                error,
            } = event
                && got == id
            {
                return (status, error);
            }
        }
        unreachable!()
    }

    /// 在 127.0.0.1:47210 上起一个接收端，需要该端口空闲（本机未运行 Local Echo）。
    #[tokio::test(flavor = "multi_thread")]
    async fn protocol_round_trip() {
        let (a, mut a_rx, a_root) = instance();
        let (b, mut b_rx, b_root) = instance();
        let listener = TcpListener::bind(("127.0.0.1", data::PORT)).await.unwrap();
        tokio::spawn(serve(b.clone(), listener));

        // hello：拿到对方设备 ID，对方被动建立联系人。
        let b_id = hello(&a, "127.0.0.1").await.unwrap();
        assert_eq!(b_id, b.device_id);
        let a_on_b = b
            .db(|db| db.contact_by_peer(&a.device_id))
            .unwrap()
            .unwrap();
        assert_eq!(a_on_b.name, a.user);

        let mut contact = Contact {
            id: 0,
            kind: ContactKind::Peer,
            name: "老李".into(),
            host: "127.0.0.1".into(),
            port: data::PORT,
            user: String::new(),
            peer_id: Some(b_id),
            remote_dir: String::new(),
            rdp_port: crate::db::DEFAULT_RDP_PORT,
        };
        contact.id = a.db(|db| db.insert_contact(&contact)).unwrap();

        // 文本
        let text = outgoing(&contact, MsgKind::Text, "你好 hello", None, None);
        a.db(|db| db.upsert_message(&text)).unwrap();
        send(a.clone(), contact.clone(), text.clone()).await;
        assert_eq!(status_of(&mut a_rx, &text.id).await.0, Status::Ok);
        let got = b.db(|db| db.message(&text.id)).unwrap().unwrap();
        assert_eq!((got.body.as_str(), got.outgoing), ("你好 hello", false));

        // 文件：3 MB，内容一致，保留原名
        let src = a_root.join("报告.bin");
        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &bytes).unwrap();
        let file = outgoing(
            &contact,
            MsgKind::File,
            "报告.bin",
            Some(a.store(&src)),
            Some(bytes.len() as u64),
        );
        a.db(|db| db.upsert_message(&file)).unwrap();
        send(a.clone(), contact.clone(), file.clone()).await;
        assert_eq!(status_of(&mut a_rx, &file.id).await.0, Status::Ok);
        assert_eq!(status_of(&mut b_rx, &file.id).await.0, Status::Ok);
        let got = b.db(|db| db.message(&file.id)).unwrap().unwrap();
        assert_eq!(got.body, "报告.bin");
        assert_eq!(
            std::fs::read(b.resolve(got.path.as_deref().unwrap())).unwrap(),
            bytes
        );

        // 图片：接收方按时间戳重新命名
        let img = a_root.join("截图.PNG");
        std::fs::write(&img, b"fake png").unwrap();
        let image = outgoing(
            &contact,
            MsgKind::Image,
            "截图.PNG",
            Some(a.store(&img)),
            Some(8),
        );
        a.db(|db| db.upsert_message(&image)).unwrap();
        send(a.clone(), contact.clone(), image.clone()).await;
        assert_eq!(status_of(&mut b_rx, &image.id).await.0, Status::Ok);
        let got = b.db(|db| db.message(&image.id)).unwrap().unwrap();
        assert!(
            got.body.ends_with(".png") && got.body.starts_with("20"),
            "{}",
            got.body
        );

        // 设备不匹配：拒收，并提示修改 IP
        let mut wrong = contact.clone();
        wrong.peer_id = Some("someone-else".into());
        let text = outgoing(&wrong, MsgKind::Text, "hi", None, None);
        a.db(|db| db.upsert_message(&text)).unwrap();
        send(a.clone(), wrong, text.clone()).await;
        let (status, error) = status_of(&mut a_rx, &text.id).await;
        assert_eq!(status, Status::Failed);
        assert!(error.unwrap().contains("当前不是「老李」的设备"));

        let _ = std::fs::remove_dir_all(a_root);
        let _ = std::fs::remove_dir_all(b_root);
    }
}
