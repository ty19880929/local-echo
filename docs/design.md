# Local Echo 详细设计方案

> 版本：**v1.1**（程序 v0.2.0 的增量，见下方「v0.2.0 变更摘要」；v1.0 定稿于 2026-11-09）　日期：2026-11-11
> 产品名称：Local Echo（`local-echo`），不设中文名。

**v0.2.0 变更摘要**（详细方案与确认记录见 [plan-v0.2.md](plan-v0.2.md)，设计稿见 `docs/ui/Local Echo v0.2.dc.html`）

| # | 调整 | 落点 |
|---|---|---|
| U1 | 文件卡片「打开」「打开文件夹」各占一半；本地文件已不存在时显示「文件已被移动或删除」 | 4.4 |
| U2 | 本机 IP 从侧边栏移到设置页第一组，表格展示，可收起 | 7.3、9 |
| U3 | 底栏高度统一为 60，侧边栏与会话区的底部分割线对齐 | 9 |
| F1 | 服务器会话新增「在远程桌面打开」（RDP；Windows 用 mstsc，Linux 用 Remmina 等） | 5.7、3.2 |
| B1 | 修复 Windows 下「打开所在文件夹」没有反应（路径分隔符混用） | 3.3、4.4 |


**v0.4 变更摘要**

| 确认项（第三轮） | 调整 |
|---|---|
| 1 同名确认与图片 | 同名确认只针对**文件**，图片不参与（第 6 节）；所有图片写入 `files/` 时，都按保存时刻的本地时间重新命名（3.3） |
| 2 开机自启隐藏启动 | 确认采纳：自启时以 `--hidden` 启动，只驻留托盘（8.4） |
| 3 编辑服务器规则 | 确认采纳（5.3、3.2） |

为了格式化本地时间（图片文件名、消息时间、同名确认中的上次发送时间），新增 `chrono` 依赖。它本来就在 gpui-kit 的依赖树中（2.2）。

第三轮确认后新提出的 1 项（图片命名细节）已确认按建议执行，见 14.4。

---

## 0. 结论速览

| 项 | 方案 |
|---|---|
| 形态 | 单个可执行程序、单窗口；关闭窗口后驻留托盘，托盘菜单或设置页的「退出」才真正退出 |
| 与人交互 | TCP 直连，固定端口 `47210`，每条消息一条短连接。联系人以设备 ID 识别，IP 只是当前通信地址（收到对方消息时自动更新），昵称和 IP 都可以编辑 |
| 与机器交互 | 纯 Rust SSH（russh + russh-sftp）。密码只在添加服务器时用一次，用来把本工具的公钥写进服务器的 `authorized_keys`；之后上传、下载、打开终端都用这把密钥。服务器有名称，可以编辑 |
| 同名文件确认 | 给联系人发文件（图片除外）和上传到服务器时都会检查：本工具记录里有同名文件发送 / 上传成功过，就弹窗告知上次时间，用户确认后才继续 |
| 图片命名 | 所有写入 `files/` 的图片都以保存时刻命名，例如 `20261109-143205-123.png` |
| 在终端中打开 | 拉起系统终端执行 `ssh -i <工具密钥> …`，免密进入当前远程目录 |
| 存储 | 一个「数据目录」：`local-echo.db`（SQLite）、`files/`、`id_ed25519`、`device_id` |
| 设置 | 本机 IP（v0.2）、数据目录（即文件保存路径，更换即迁移）、开机自动启动、退出程序 |
| 在远程桌面打开 | v0.2：探测服务器的 RDP 端口后拉起本机客户端（Windows：mstsc；Linux：Remmina 等），密码由客户端询问，不保存 |
| 托盘 | Windows 和 Linux 统一使用 `tray-icon`（Linux 走 KSNI，不依赖 GTK）；有未读消息时图标带圆点 |
| 发布 | 推送 `v*` 标签后，GitHub Actions 构建 Windows 安装程序（Inno Setup）和 Linux 的 deb / rpm |

---

## 1. 需求覆盖矩阵

| # | 需求 | 设计落点 | 章节 |
|---|---|---|---|
| F1 | 两大类场景：与人交互、与机器交互 | 联系人分两类：`kind=0` 人，`kind=1` 机器；侧边栏分为「联系人」「服务器」两组 | 3.2、9 |
| F2 | 点对点，没有中转或服务端 | 人：TCP 直连；机器：SSH 直连 | 4、5 |
| F3 | 两类场景都用聊天窗口 | 两类会话共用一个聊天视图 | 9 |
| F4 | 人：文本 | `text` 消息 | 4.4 |
| F5 | 人：表情 | **按第一轮确认移除** | — |
| F6 | 人：图片 | `image` 消息：可粘贴、选择或拖拽发送，在聊天中内联显示 | 4.4 |
| F7 | 人：文件传输 | `file` 消息：流式传输，两端都显示进度 | 4.4 |
| F8 | 人：通过指定 IP 添加对端 | 「添加联系人」：填写 IP 和昵称，然后 `hello` 握手 | 4.3 |
| F9 | 人：两端都需要安装本工具 | 对端需要运行本工具并监听 `47210` | 4.1 |
| F10 | 机器：从对端下载（对方给我发消息） | 在远程浏览器中选择文件，SFTP 下载，显示为左侧消息 | 5.5 |
| F11 | 机器：向对端上传（我给对方发消息） | 选择或拖入文件，SFTP 上传，显示为右侧消息 | 5.5 |
| F12 | 机器：在终端中打开 | 拉起系统终端，用 ssh 加工具密钥登录 | 5.6 |
| F13 | 机器：通过 IP / 用户名 / 密码添加 | 密码登录 → 部署公钥 → 用密钥复验 | 5.2 |
| F14 | 机器：对端不需要安装本工具，走 SSH | 只依赖标准 sshd 和 SFTP | 5 |
| F15 | 设置：文件保存路径 | 「数据目录」 | 7.1 |
| F16 | 设置：数据迁移 | 更换数据目录即迁移；换电脑时拷贝目录后选用 | 7.2 |
| C1-8 | 添加时设置昵称；可以编辑昵称和 IP；不以 IP 作为唯一标识 | 设备 ID + 本地昵称 | 4.1、4.3 |
| C1-9 | 托盘常驻，不需要系统通知 | `QuitMode::Explicit`、tray-icon、单实例唤起 | 8 |
| C1-11 / C2-2 / C2-3 / C3-1 | 同名文件确认：联系人和服务器都要，只针对文件，只依据本工具的记录，并告知上次时间 | 第 6 节 | 6 |
| C3-1 | 图片保存时按时间戳重命名 | 图片命名规则 | 3.3 |
| C3-2 | 开机自启时不弹出主窗口 | `--hidden` | 8.4 |
| C3-3 | 编辑服务器规则 | 直接保存、名称必填、不要求唯一 | 5.3、3.2 |
| C1-13 | Windows 和 Linux 都通过安装包安装 | Inno Setup / deb / rpm | 11 |
| C1-15 | 删除联系人及聊天记录，保留已传输的文件 | 级联删除消息 | 4.3、5.3 |
| C2-1 | 对端 IP 自动更新 | 收到已知设备的请求时更新 IP | 4.1 |
| C2-4 | 托盘未读提示 | 托盘图标带圆点 + 悬停提示 | 8.2 |
| C2-6 | 设置页「开机自动启动」，下次启动生效 | 注册表 Run 键 / XDG autostart | 8.4 |
| C2-8 | 服务器名称与编辑 | `contact.name`、编辑服务器对话框 | 5.3 |
| T1 | Rust + gpui（gpui-kit） | gpui-kit 0.9.1 | 2.2 |
| T2 | 用 SQLite 存记录；文件和图片不入库，库里只存路径 | 3.2、3.3 | 3 |
| T3 | 用 GitHub Actions 发布 Windows / Linux 版本 | `release.yml` | 11 |
| U1 | Claude Design 提示词 | 附录 A | 附录 A |

C1-x、C2-x、C3-x 分别表示第一、二、三轮确认中的第 x 项。

**不做**（需求没有提到，或已确认不做）：表情、在线状态、局域网自动发现、群聊、已读回执、撤回或编辑消息、搜索、系统通知、取消传输、断点续传、文件夹传输、加密传输、主机指纹校验、自动更新。

---

## 2. 总体架构

### 2.1 进程与线程模型

```
┌────────────────────────── local-echo（单进程、单实例）──────────────────────────┐
│                                                                                 │
│  gpui 主线程（UI，QuitMode::Explicit）          tokio 运行时（全部网络 I/O）     │
│  ┌──────────────────────┐   spawn 任务         ┌───────────────────────────┐   │
│  │ 主窗口（可关闭、重建）│ ───────────────────▶ │ peer：监听 47210、发送    │   │
│  │ 对话框 / 设置         │                      │ host：SSH / SFTP          │   │
│  └──────────▲───────────┘   Event 通道         └─────────────┬─────────────┘   │
│             ├───────────────────────────────────────────────┘                 │
│             └──── 托盘事件（tray-icon 回调转发到同一个通道）                     │
│                                                                                 │
│  数据目录：local-echo.db  files/  id_ed25519  device_id                          │
└─────────────────────────────────────────────────────────────────────────────────┘
        │ TCP 47210（直连联系人）                     │ SSH（直连服务器）
```

- **tokio**：在 `main` 中创建多线程运行时，把 `Handle` 放入全局状态。UI 通过 `handle.spawn(...)` 发起网络操作；需要结果时直接 `await` 返回的 `JoinHandle`（它是普通 `Future`，可以跨执行器 await）。
- **事件通道**：网络任务和托盘回调都只通过一个 `tokio::sync::mpsc::UnboundedSender<Event>` 通知 UI。gpui 前台用一个任务循环执行 `rx.recv().await`，这条链路已实测可用。
- **数据库**：共享一个 `Arc<Mutex<rusqlite::Connection>>`，开启 WAL。
- **主窗口**只负责展示。联系人、未读数、当前会话等状态都放在全局状态里，所以窗口可以随时关闭和重建。

```rust
enum Event {
    MessageAdded(Message),                  // 收到新消息，或新建了一条传输
    Progress { id: String, done: u64 },     // 传输进度，发送端最多每 100ms 发一次
    Status { id: String, status: Status },  // 进行中 → 成功 / 失败
    ContactChanged(Contact),                // 新联系人，或 IP 被自动更新
    ShowWindow,                             // 托盘单击、托盘菜单「打开」、第二个实例要求唤起
    Quit,                                   // 托盘菜单「退出」
}
```

### 2.2 技术选型（版本于 2026-11-09 实测）

| 用途 | 选型 | 理由 |
|---|---|---|
| UI | `gpui-kit = "=0.9.1"`（固定 `gpui-pre 0.3.12`） | 技术栈指定；自带 `Message`、`Bubble`、`Attachment`、`MessageScroller`、`Sidebar`、`Dialog`、`Textarea`（`on_paste`）、`Setting`、`Switch`、`TitleBar` |
| 异步运行时 | `tokio` | russh 依赖它，网络 I/O 统一放在上面 |
| SSH / SFTP | `russh 0.65`（关闭默认特性，启用 `ring`、`rsa`、`flate2`）+ `russh-sftp 3` | 纯 Rust 实现，不依赖 OpenSSL；用 `ring` 后 Windows CI 不需要 NASM / CMake |
| 密钥生成 | `rand 0.10` + `russh::keys::PrivateKey::random(Ed25519)` | — |
| 托盘 | `tray-icon 0.26`（关闭默认特性，启用 `ksni`） | 一个 crate 覆盖两个平台，Linux 下不依赖 GTK；支持运行时 `set_icon` / `set_tooltip` |
| 托盘图标解码 | `image 0.25`（只启用 `png`） | 已在 gpui 的依赖树里 |
| 开机自启（Windows） | `winreg 0.56`（仅 Windows） | 读写 `HKCU\…\Run`。Linux 只需写一个文件，不需要依赖 |
| 本地时间格式化 | `chrono 0.4` | 用于图片文件名、消息时间、同名确认中的上次时间；已在 gpui-kit 的依赖树中（含 `clock` 特性），不增加编译量 |
| 数据库 | `rusqlite 0.41`（`bundled`） | 静态链接 SQLite |
| 其他 | `serde`、`serde_json`、`uuid`、`dirs`、`local-ip-address`、`anyhow` | 各自只承担单一用途 |
| 打包 | Windows：Inno Setup 6（runner 已预装）+ `winresource`；Linux：`cargo-deb`、`cargo-generate-rpm` | 见第 11 节 |

---

## 3. 数据存储

### 3.1 数据目录

```
<数据目录>/
├── local-echo.db      # SQLite：联系人 + 消息
├── device_id          # 本机设备 ID（UUID，纯文本），首次启动时生成
├── id_ed25519         # 工具专用 SSH 私钥，首次添加服务器时生成
└── files/             # 收到 / 下载的文件和图片，以及我发出的图片（扁平存放；文件保留原名，重名时追加 " (1)"；图片按时间戳命名）
```

- 默认位置：Linux 为 `~/.local/share/local-echo`，Windows 为 `%LOCALAPPDATA%\local-echo`。
- 指针文件：`<系统配置目录>/local-echo/data_dir`，只有一行绝对路径；文件不存在时使用默认位置。
- 「开机自动启动」不存在数据目录里。它的状态以系统中的自启项是否存在为准（8.4）。

### 3.2 表结构

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS contact (
  id         INTEGER PRIMARY KEY,
  kind       INTEGER NOT NULL,            -- 0 = 人（联系人），1 = 机器（服务器）
  name       TEXT    NOT NULL,            -- 联系人昵称 / 服务器名称（本地设置，可编辑）
  host       TEXT    NOT NULL,            -- 人：对端当前 IP；机器：服务器地址
  port       INTEGER NOT NULL,            -- 人：47210；机器：SSH 端口
  user       TEXT    NOT NULL DEFAULT '', -- 机器：SSH 用户名
  peer_id    TEXT,                        -- 人：对端设备 ID（唯一标识）
  remote_dir TEXT    NOT NULL DEFAULT ''  -- 机器：当前远程目录
);
CREATE UNIQUE INDEX IF NOT EXISTS contact_peer ON contact (peer_id) WHERE kind = 0;

CREATE TABLE IF NOT EXISTS message (
  id          TEXT    PRIMARY KEY,        -- UUID；人场景下两端一致，用于重试去重
  contact_id  INTEGER NOT NULL REFERENCES contact(id) ON DELETE CASCADE,
  outgoing    INTEGER NOT NULL,           -- 1 = 我发出（右侧），0 = 对方发来（左侧）
  kind        INTEGER NOT NULL,           -- 0 = 文本，1 = 图片，2 = 文件
  body        TEXT    NOT NULL,           -- 文本内容或文件名
  path        TEXT,                       -- 本地文件路径（规则见 3.3）
  remote_path TEXT,                       -- 机器：远端完整路径
  size        INTEGER,
  status      INTEGER NOT NULL,           -- 0 = 进行中，1 = 成功，2 = 失败
  ts          INTEGER NOT NULL            -- 本地时间（毫秒）
);
CREATE INDEX IF NOT EXISTS message_by_contact ON message (contact_id);
```

- **结构版本（v0.2 起）**：用 `PRAGMA user_version` 记录。打开数据库后先按上面的 v0.1 结构建表，再逐级升级，新库也走同一条路径：
  - `user_version < 1`：先用 `pragma_table_info('contact')` 检查有没有 `rdp_port` 列，没有就执行 `ALTER TABLE contact ADD COLUMN rdp_port INTEGER NOT NULL DEFAULT 3389`（机器：远程桌面端口；联系人不使用），然后把 `user_version` 设为 1。
  - 只做加列。v0.1 的 SELECT / INSERT 都写了明确的列名，v0.1 程序打开 v0.2 的数据库不受影响，降级和换电脑迁移都没问题。
- 服务器条目只以 `id` 区分，不要求「地址 + 用户名」唯一（已确认）。
- 消息按本地 `rowid` 排序显示。启动时执行 `UPDATE message SET status = 2 WHERE status = 0`，把上次未完成的传输标为失败。
- 删除联系人或服务器：执行 `DELETE FROM contact WHERE id = ?`，消息随之级联删除；`files/` 里的文件不动。

### 3.3 文件与图片存放规则

| 场景 | 实际存放 | `message.path` |
|---|---|---|
| 收到的文件（人） | `files/<原文件名>` | 相对路径 |
| 收到的图片（人） | `files/<时间戳>.<扩展名>` | 相对路径 |
| 从服务器下载的文件 | `files/<原文件名>` | 相对路径 |
| 我发出的图片（选择、拖入或粘贴） | 复制到 `files/<时间戳>.<扩展名>`（粘贴的图片扩展名为 `.png`） | 相对路径 |
| 我发出的文件（人）、上传的文件（机器） | 不复制 | 原文件的绝对路径 |

- **图片命名（C3-1）**：所有写入 `files/` 的图片都用保存时刻的本地时间命名，格式为 `YYYYMMDD-HHMMSS-mmm.<扩展名>`，例如 `20261109-143205-123.png`，用 chrono 格式化。发送方在复制时命名，接收方在接收时按自己的时间命名，两边各自独立（已确认）。同一毫秒内重名时，也按下面的规则追加序号。图片消息的 `body` 记录的就是这个保存后的文件名。
- **文件名处理**：只取路径的最后一段；Windows 下把 `\/:*?"<>|` 替换为 `_`；重名时追加 ` (1)`、` (2)`……
- **相对路径的还原（v0.2 B1）**：相对路径统一按 `/` 存储；读取时按 `/` 切开逐段拼接，得到全部使用本机分隔符的路径。Windows 的 Shell 不接受混用 `/` 的路径，直接 `join` 会让「打开文件夹」失败。

---

## 4. 与人交互

### 4.1 身份与寻址

- **设备 ID**：每个安装实例首次启动时生成一个 UUID，存放在 `<数据目录>/device_id`。**联系人以对端的设备 ID 唯一标识**；IP 只是当前通信地址。所有实例都监听 `0.0.0.0:47210`。
- **昵称**：添加联系人时设置，之后可以编辑；只保存在本地。界面显示为「昵称 · IP」。
- **被动建立联系人**：收到未知设备的请求时，自动新建联系人。默认昵称为对方的系统用户名（协议字段 `user`），IP 为本次连接的来源 IP。
- **IP 自动更新（已确认）**：收到已知设备的请求，且来源 IP 与记录不同时，把记录更新为来源 IP。
- **防止发错人**：每个请求都携带目标设备 ID `to`。接收方发现不是自己就拒收，发送方提示「发送失败：{IP} 当前不是「{昵称}」的设备，请修改 IP」。

### 4.2 传输协议

一条消息一条 TCP 短连接：连接 → 写请求 → 读响应 → 关闭。

```
请求：[u32 大端 N][N 字节 JSON 头][size 字节文件内容（仅 image / file）]
响应：[u32 大端 M][M 字节 JSON]
```

| 请求头字段 | 说明 |
|---|---|
| `t` | `hello` / `text` / `image` / `file` / `show`（`show` 见 8.3） |
| `from` | 发送方设备 ID |
| `to` | 接收方设备 ID（`hello`、`show` 没有） |
| `user` | 发送方系统用户名，只用作对方自动建立联系人时的默认昵称 |
| `id` | 消息 UUID |
| `text` | 文本内容 |
| `file`、`size` | 文件名、字节数。图片只用到文件名中的扩展名，接收方按时间戳重新命名 |

响应：成功为 `{"ok":true}`，`hello` 的响应额外带上 `"id":"<接收方设备 ID>"`；失败为 `{"ok":false,"err":"wrong_peer" | "<原因>"}`。

**接收方处理顺序**：
1. `t = show`：只接受来自本机回环地址的请求，唤起窗口。
2. `to` 存在且不等于本机 ID：返回 `wrong_peer`。
3. 按 `from` 查找联系人：不存在则新建，IP 变了则更新。
4. 按 `t` 处理。

**超时**：连接 3 秒；传输过程中连续 30 秒没有任何进展即判定失败。

### 4.3 添加、编辑、删除联系人

```mermaid
sequenceDiagram
  participant A as 我（A）
  participant B as 联系人（B）
  A->>A: 输入 IP 和昵称「老李」
  A->>B: 连接 B:47210，{t:"hello", from:A_id, user:"zhangsan"}
  B->>B: 按 A_id 新建或更新联系人（默认昵称 zhangsan，IP 取来源 IP）
  B-->>A: {ok:true, id:B_id}
  A->>A: 按 B_id 新建联系人（昵称「老李」），打开会话
```

- **添加**：IP 和昵称都必填。
  - 连接失败时提示「无法连接 {IP}：对方未运行 Local Echo，或网络不可达」，不添加。
  - 返回的设备 ID 已在列表中时，更新该联系人的 IP 和昵称并打开会话，提示「该联系人已在列表中，已更新 IP 和昵称」。
- **编辑**（右键 →「编辑」）：可修改昵称和 IP，直接保存，不做连接校验。
- **删除**（右键 →「删除」，需二次确认）：删除联系人及其聊天记录，保留已传输的文件。

### 4.4 收发消息

| 类型 | 发送入口 | 接收端处理 | 显示 |
|---|---|---|---|
| 文本 | 输入框：Enter 发送，Shift+Enter 换行 | 入库 | 气泡，文字可选中复制 |
| 图片 | ① 在输入框粘贴图片 ② 点附件按钮选择 ③ 拖到聊天区。扩展名为 `png/jpg/jpeg/gif/webp/bmp` 时按图片处理 | 流式写入 `files/<时间戳>.<扩展名>` | 内联缩略图，最大 240×240、保持比例；点击后用系统查看器打开 |
| 文件 | ① 附件按钮（可多选）② 拖到聊天区 ③ 粘贴已复制的文件 | 流式写入 `files/<原文件名>`，自动接收 | `Attachment` 卡片；完成后可「打开」「打开文件夹」 |

- 文件在发出前要先经过**同名文件确认**（第 6 节）；图片不需要。
- **传输流程**：
  1. 发送端插入一条 `status = 0` 的消息。
  2. 连接对端，写入请求头，再按 256 KB 分块写入内容，同时发送进度。
  3. 接收端执行 `INSERT … ON CONFLICT(id) DO UPDATE SET status = 0`，把内容流式写入 `files/`，完成后标记成功并回复 `ok`；出错时删除半成品文件，标记失败。
- **失败与重试**：显示「发送失败 · 重试」。重试使用同一个 id，接收方按 id 覆盖，不会出现重复消息；重试不触发同名确认。
- **文件卡片操作行（v0.2 U1、B1）**：完成后「打开」「打开文件夹」各占卡片宽度的一半，文字居中，悬停时所在半块显示 secondary 底色。联系人、服务器卡片，发出、收到、上传、下载四种卡片共用同一段代码。点击时先检查文件是否存在；「打开」用系统默认程序，「打开文件夹」在 Windows 上自己调用 `SHParseDisplayName` + `SHOpenFolderAndSelectItems`（路径已统一为 `\`），失败时退回 `explorer /select,"<路径>"`，两者都失败才报错（gpui 的 `reveal_path` 在 Windows 上失败时只写日志，用户看不到任何反馈）；不直接用 explorer，是因为该目录已开着资源管理器窗口时，Windows 11 只激活那个窗口、不选中文件（2026-11-11 在测试机上发现，已确认改用系统 API）。Linux 上沿用 gpui 的 `reveal_path`。出错时弹出错误提示：「无法打开文件夹：{原因}」或「文件不存在，可能已被移动或删除」。
- **文件已不存在（v0.2 #13）**：传输成功、但本地文件后来被移动、改名或删除时，在操作行的位置显示「文件已被移动或删除」（muted 色，13 px，与操作行等高，卡片高度不变）。收到 / 下载的文件，以及我发出 / 上传的文件（指向原文件）都适用；图片文件丢失时退化为文件卡片显示。渲染卡片、窗口重新获得焦点、点击「打开」时都会重新检查。不提供「重新定位文件」之类的功能。
- **未读**：收到联系人发来的消息时，如果主窗口已关闭，或者该会话不是当前打开的会话，就给这个联系人的未读数加一（只保存在内存中）。侧边栏显示每个联系人的未读数，托盘显示总数（8.2）。打开会话即清零。不发系统通知。

---

## 5. 与机器交互

### 5.1 认证方案（已确认）

本工具持有一把 ed25519 密钥 `<数据目录>/id_ed25519`。添加服务器时用密码登录一次，把公钥追加到服务器的 `~/.ssh/authorized_keys`（注释为 `local-echo@<本机名>`）。此后所有操作都用这把密钥，密码不落盘。删除服务器时不清理这行公钥。

### 5.2 添加服务器

```mermaid
sequenceDiagram
  participant U as 本工具
  participant S as 服务器 sshd
  U->>S: SSH 连接（地址 / 端口，默认 22），密码认证
  Note over U,S: password 方式失败、而服务器提供 keyboard-interactive 时，<br/>用同一个密码应答
  U->>U: 首次使用时生成 id_ed25519（Linux：0600；Windows：icacls 限定为当前用户）
  U->>S: SFTP：确保 ~/.ssh 存在（0700），公钥不存在时追加到 authorized_keys（0600）
  U->>S: 断开，用密钥重新登录进行复验
  U->>S: SFTP canonicalize(".") 得到主目录，作为初始远程目录
  U->>U: 写入服务器（名称、地址、端口、用户名、远程目录；不含密码），打开会话
```

- **字段**：名称、地址（`IP[:端口]`，默认端口 22）、用户名、密码，都必填。
- **报错**：
  - 「无法连接 {host}:{port}」
  - 「用户名或密码错误」
  - 「公钥已写入，但无法用密钥登录：服务器可能禁用了公钥认证」

  出现任意一种都不会添加服务器。
- **重新授权**：密钥登录失败时，会话顶部显示横幅「无法登录该服务器，访问密钥可能已失效」和「重新授权」按钮。点击后打开授权对话框，名称、地址和用户名已预填且只读，只需填写密码，之后按上图流程重新部署公钥。
- **主机指纹**：不校验（已确认）。

### 5.3 编辑与删除服务器（C2-8）

- **编辑**（右键 →「编辑」）：可修改名称、地址（`IP[:端口]`）、用户名，以及远程桌面端口（v0.2，见 5.7），直接保存，不做连接校验，远程目录保持不变。
  - 如果新的地址或用户名上没有部署过本工具的公钥，下次操作会触发「重新授权」横幅，填一次密码即可。
  - 修改地址或用户名后，会丢弃该服务器缓存的 SFTP 连接；只改远程桌面端口不影响 SSH 连接。
- **删除**（右键 →「删除」，需二次确认）：删除服务器及其传输记录，保留已下载的文件。

### 5.4 远程目录

每台服务器有一个**当前远程目录**（`contact.remote_dir`），显示在会话顶部。上传、浏览下载、在终端中打开都基于这个目录。

**远程浏览器**（对话框）有两种模式：

| 模式 | 打开方式 | 底部按钮 |
|---|---|---|
| 选择目录 | 点击顶部的「更改」 | 「选择此目录」 |
| 选择文件 | 点击「下载文件」 | 可多选，「下载所选」 |

- 界面元素：路径栏（可直接输入路径后回车跳转）、「上一级」按钮、文件列表（图标 / 名称 / 大小 / 修改时间，目录排在前面，双击进入目录）。
- 数据来源：SFTP `read_dir`。遇到符号链接时再取一次 `metadata`，判断它指向的是否为目录。

### 5.5 上传与下载

| 操作 | 入口 | 消息方向 | 本地 / 远端 |
|---|---|---|---|
| 上传（我给对方发消息） | 「上传文件」按钮（可多选），或把文件拖到会话区；**先经过同名文件确认**（第 6 节） | 右侧 | 原文件 → `<remote_dir>/<文件名>` |
| 下载（对方给我发消息） | 「下载文件」→ 在远程浏览器中多选 | 左侧 | 远端文件 → `files/<文件名>` |

- 每个文件是一条消息，用 `Attachment` 卡片显示进度和状态；按 256 KB 分块读写。
- SFTP 连接复用：每台服务器缓存一个 `Arc<SftpSession>`，首次使用时用密钥建立，之后各操作并发复用。任何操作出错就丢弃缓存，下次操作时重新连接。

### 5.6 在终端中打开（已确认）

```
ssh -i <数据目录>/id_ed25519 -p <port>
    -o IdentitiesOnly=yes -o StrictHostKeyChecking=no
    -o UserKnownHostsFile=<Linux: /dev/null | Windows: NUL> -o LogLevel=ERROR
    -t <user>@<host> "cd '<remote_dir>'; exec $SHELL -l"
```

| 平台 | 拉起方式 |
|---|---|
| Windows | `Command::new("ssh").creation_flags(CREATE_NEW_CONSOLE)`，使用系统自带的 OpenSSH 客户端 |
| Linux | 优先使用 `x-terminal-emulator`：解析其软链接得到真实终端，再按下表组装参数。没有它时按下表顺序查找；全都找不到时提示「未找到可用的终端程序」 |

| Linux 终端 | 命令形式 |
|---|---|
| ptyxis | `ptyxis --new-window -- ssh …` |
| gnome-terminal | `gnome-terminal -- ssh …` |
| konsole | `konsole -e ssh …` |
| xfce4-terminal | `xfce4-terminal -x ssh …` |
| kitty / foot | `kitty ssh …` / `foot ssh …` |
| alacritty | `alacritty -e ssh …` |
| wezterm | `wezterm start -- ssh …` |
| xterm | `xterm -e ssh …` |

私钥权限在两个时机设置：生成密钥时、迁移数据目录后。Linux 用 `chmod 600`；Windows 用 `icacls <key> /inheritance:r /grant:r "%USERNAME%:F"`。

### 5.7 在远程桌面打开（v0.2 F1）

用本机的远程桌面客户端以 RDP 协议连接这台服务器。`mstsc` 只支持 RDP，Linux 上主流客户端也都支持 RDP，两个平台统一。

- **前提**：服务器需要自己开启 RDP 服务（Windows 远程桌面、Linux 的 xrdp / GNOME 远程桌面 / KDE KRdp），本工具不负责安装或配置。
- **地址**：服务器的 SSH 地址（`contact.host`）；端口默认 3389，可在「编辑服务器」中修改（字段「远程桌面端口」，选填，留空按 3389，范围 1–65535，超出时提示「请输入 1–65535 之间的端口」）。「添加服务器」「重新授权」不变。
- **用户名**：预填该服务器的 SSH 用户名，用户可以在客户端的登录框里改，不另设字段。
- **密码不保存**，与 5.1「密码不落盘」一致。RDP 用不了本工具的 SSH 密钥，密码由客户端询问；「记住密码」交给客户端（mstsc 存进 Windows 凭据管理器，Remmina 存进系统钥匙环）。
- **与 SSH 密钥无关**：密钥失效不影响远程桌面，「重新授权」横幅也不影响这个按钮。

**流程**：点击「在远程桌面打开」→ 按钮进入「正在连接…」（Spinner，不可重复点击）→ TCP 探测 `host:rdp_port`，最多 3 秒 →

| 结果 | 反馈 |
|---|---|
| 端口不通 | 错误提示「无法连接 {host}:{port}：服务器未开启远程桌面，或端口不可达。可在「编辑服务器」中修改远程桌面端口」 |
| 拉起客户端失败 | 错误提示「无法启动远程桌面客户端：{原因}」 |
| Linux 找不到客户端 | 对话框「未找到远程桌面客户端」+ 三行安装命令（见下） |
| 成功 | 按钮恢复，不弹成功提示（客户端窗口本身就是反馈） |

先探测端口，是因为客户端拉起后的失败提示五花八门、有的要等很久才超时；「服务器没开远程桌面」这种最常见的错误在本工具里就能用中文说清楚。

**Windows**：每次覆盖写入 `%TEMP%\local-echo\rdp-<contact_id>.rdp`，再执行 `mstsc <文件>`（`mstsc /v:` 不能预填用户名）。文件与 mstsc 自己保存的格式一致：UTF-16LE 带 BOM、CRLF，中文用户名不乱码；只写两行，不含密码，其余设置用 mstsc 默认值（证书警告保持默认，不写 `authentication level`；不用 `cmdkey`）：

```
full address:s:<host>:<rdp_port>
username:s:<user>
```

- **Windows 的 `.rdp` 文件安全警告**（2026 年 4 月安全更新 CVE-2026-26151 引入，已确认保持 `.rdp` 方案）：第一次打开 `.rdp` 文件时弹出一次性说明，需勾选「我了解，并允许在此设备上为我的帐户打开 RDP 文件」；之后每次都会先弹「警告：未知的远程连接」（未签名文件），点「连接」继续，剪贴板等本地资源重定向默认不勾选、只对本次连接有效。直接 `mstsc /v:` 不会弹警告，但不能预填用户名。
- **记住凭据**：mstsc 的「记住我」只在服务器启用 NLA 时出现（Windows 远程桌面）。xrdp 不支持 NLA，用户名预填在 xrdp 自己的登录框里，密码在那里输入。

**Linux**：按顺序探测（`which` 在 PATH 中查找），用第一个可用的：

| 顺序 | 客户端 | 探测 | 拉起命令 |
|---|---|---|---|
| 1 | Remmina | `remmina` | `remmina -c rdp://<user>@<host>:<port>` |
| 2 | Remmina（Flatpak） | `flatpak` 存在且 `flatpak info org.remmina.Remmina` 成功 | `flatpak run org.remmina.Remmina -c rdp://…` |
| 3 | KRDC | `krdc` | `krdc rdp://<user>@<host>:<port>` |
| 4 | FreeRDP 3 SDL 客户端 | `sdl-freerdp3`，其次 `sdl-freerdp` | `sdl-freerdp3 /v:<host>:<port> /u:<user> /t:<服务器名称> /dynamic-resolution +clipboard` |
| 5 | GNOME Connections（Fedora Workstation 默认自带） | `gnome-connections` | `gnome-connections rdp://<host>:<port>`（只接受一个参数，不带用户名，由它的认证框询问） |
| 6 | 系统默认的 `rdp://` 处理程序 | `xdg-open` 存在且 `xdg-mime query default x-scheme-handler/rdp` 有结果 | `xdg-open rdp://<user>@<host>:<port>` |

- 不采用 `xfreerdp`：只能在终端里输入凭据，从图形程序拉起时没有终端。GNOME Connections 从命令行传入 `rdp://` 能直接连接，2026-11-11 实测通过后加入（R14，见 12 #25）。
- 默认处理程序只作兜底：Remmina 注册的处理程序执行的是不带 `-c` 的 `remmina rdp://…`，直接调用 `remmina -c` 行为更确定。
- **URI**：`rdp://<user>@<host>:<port>`；IPv6 地址加方括号；用户名含 `@ : / ? # %` 时省略用户名部分，由客户端询问，避免各客户端解码方式不同导致登录用户错误。
- **找不到客户端**：对话框（480 宽）「未找到远程桌面客户端」，说明「需要安装一个支持 RDP 的远程桌面客户端，推荐 Remmina。」，三组命令（发行版名称 + 等宽命令框，每行可复制，只复制命令本身）：`sudo apt install remmina remmina-plugin-rdp`（Ubuntu / Debian）、`sudo dnf install remmina remmina-plugins-rdp`（Fedora）、`flatpak install flathub org.remmina.Remmina`（Flatpak）；按钮「知道了」。只给安装提示，不退回到「在终端里运行 xfreerdp」。
- **打包**：deb 声明 `Suggests: remmina, remmina-plugin-rdp | krdc | freerdp3-sdl`，rpm 声明 `Suggests: remmina`。只用 Suggests（apt 默认不装），不给每个用户都装上 Remmina 及其 GTK 依赖。

---

## 6. 同名文件确认（C1-11、C2-2、C2-3、C3-1）

**触发时机**：发出**新的文件**时，即：
- 联系人会话：通过附件按钮、拖入、粘贴已复制的文件这三种方式发出、且被判定为文件的内容；
- 服务器会话：「上传文件」按钮、拖入。

以下情况不触发：图片（已确认，图片保存时会按时间戳重命名，见 3.3）；重试失败的消息。

**判断依据**：只看本工具的成功记录（已确认）：

```sql
SELECT ts FROM message
WHERE contact_id = ? AND outgoing = 1 AND status = 1 AND <同名条件>
ORDER BY rowid DESC LIMIT 1;
```

| 场景 | 同名条件 | 含义 |
|---|---|---|
| 联系人 | `kind = 2 AND body = ?`（文件名） | 曾向该联系人成功发送过同名文件 |
| 服务器 | `remote_path = ?`（`<remote_dir>/<文件名>`） | 曾成功上传到同一个远程路径 |

一次选择或拖入中同时有图片和文件时，图片直接发送，只有同名的文件进入确认弹窗。

**确认弹窗**：两个场景用同一个对话框组件，只是文案不同。同一批文件合并到一个弹窗中处理；都没有同名记录时直接发送，不弹窗。

| | 联系人 | 服务器 |
|---|---|---|
| 标题 | 已发送过同名文件 | 已上传过同名文件 |
| 列表（每个同名文件一行） | {文件名} — 上次发送于 {时间} | {文件名} — 上次上传于 {时间} |
| 说明 | 继续发送后，对方会再收到一份。 | 继续上传将覆盖服务器上 {remote_dir} 中的同名文件。 |
| 按钮 | 取消（整批不发送）/ 继续发送（整批发送） | 取消 / 继续上传 |

---

## 7. 设置

设置页的内容：

| 设置项 | 控件 | 说明 |
|---|---|---|
| 本机 IP（v0.2，第一组） | 可收起的表格：网卡 / IP 地址 / 复制 | 7.3 |
| 数据目录 | 路径 +「打开」「更改…」 | 7.1、7.2 |
| 开机自动启动 | 开关 | 8.4 |
| 退出 Local Echo | 按钮 | 8.1 |
| 关于 | 版本号 | — |

v0.2 起本机 IP 不再显示在侧边栏顶部，移到设置页第一组（7.3）。

### 7.1 数据目录（即文件保存路径）

显示当前路径，旁边有「打开」（在文件管理器中打开）和「更改…」两个按钮。

### 7.2 数据迁移

| 目标目录 | 行为 |
|---|---|
| 已包含 `local-echo.db` | 确认「使用该目录中的已有数据？当前数据将保留在原位置。」→ 改写指针文件 → `cx.restart()` |
| 不包含 | 确认「将全部数据移动到 X？」→ 关闭数据库 → 移动 `local-echo.db`、`device_id`、`id_ed25519`、`files/`（同一分区直接 `rename`，跨分区先复制再删除，并显示进度）→ 修正私钥权限 → 改写指针文件 → `cx.restart()` |

- 换电脑时，把整个数据目录拷到新电脑，在设置里「更改…」选中它即可。
- 有传输正在进行时，提示「有传输正在进行，请完成后再迁移」。

### 7.3 本机 IP（v0.2 U2）

- **位置**：设置页第一组，在「数据目录」之前。IP 不在侧边栏后，用户来设置页大多是为了查看它。
- **表格**：列「网卡」「IP 地址」「操作」。网卡名过长时单行中间省略，被省略时悬停显示全名；IP 用等宽字体；「操作」是复制图标按钮。表头、行线与远程浏览器共用 `kit::table_head` / `kit::table_row`。
- **复制**：点击复制图标，或双击 IP：写入剪贴板，提示「已复制 {ip}」，该行图标在 2 秒内显示为对勾。
- **收起 / 展开**：标题行右侧有 chevron，整行可点击。收起时卡片内只显示一行摘要「{第一个 IP} 等 {n} 个」（只有 1 个时只显示 IP）。默认展开；用户的选择只在本次运行期间记住（`AppState`），不写盘。
- **说明**：卡片底部「联系人添加你时，填写其中任意一个 IP 即可（端口固定为 47210）」。没有可用地址时显示「未检测到可用的网络连接」。
- **数据来源**：`net::local_ips()`，每次打开设置页、每次展开时重新读取，换 Wi-Fi、连 VPN 后不用重启程序。只列 IPv4、非回环地址，并过滤 169.254.0.0/16（Windows 上断开的网卡常显示这个自动分配地址，填了也连不上）。
- **排序**：物理网卡在前（默认路由所在的排第一），虚拟网卡（vEthernet、docker、br-、VPN / 代理隧道等）在后，其余保持系统给出的顺序；收起时的摘要取第一行。默认路由由 UDP 套接字的 `connect` 选路得到（不发送任何数据包）。开着代理 TUN 时默认路由指向 TUN 网卡，它对局域网联系人没有用，所以「虚拟」的判断优先于默认路由；198.18.0.0/15（Clash / Mihomo 的 fake-ip 网段）也按虚拟网卡处理。

---

## 8. 托盘常驻、单实例与开机自启

### 8.1 托盘常驻

| 行为 | 实现 |
|---|---|
| 关闭主窗口 | 以 `QuitMode::Explicit` 运行。关闭窗口只是关掉窗口，监听和传输继续进行 |
| 打开主窗口 | 托盘左键单击（Windows）/ 激活（Linux），或托盘菜单「打开 Local Echo」，发出 `Event::ShowWindow`。窗口不存在则 `open_window` 重建，存在则 `activate_window` |
| 退出 | 托盘菜单「退出」，或设置页「退出 Local Echo」，执行 `cx.quit()` |

- **托盘实现**：使用 `tray-icon`。Windows 上是 Shell_NotifyIcon，Linux 上是 KSNI（StatusNotifierItem）。菜单和点击事件通过 `set_event_handler` 转发到事件通道。图标是内嵌的 PNG，用 `image` 解码后传给 `Icon::from_rgba`。
- **没有托盘宿主的 Linux 桌面**（已确认）：不做探测，行为保持一致。关闭窗口后程序驻留后台；再次启动程序会唤起窗口；可以在设置页退出。
- 窗口的 `app_id` 为 `local-echo`，与 `.desktop` 文件中的 `StartupWMClass` 一致。

### 8.2 托盘未读提示（C2-4）

- 所有联系人的未读数之和大于 0 时：调用 `tray.set_icon(带圆点的图标)` 和 `tray.set_tooltip("Local Echo · {n} 条未读")`。
- 未读数归零时：恢复常规图标，提示文字恢复为 `Local Echo`。
- 未读数的计算规则见 4.4。服务器的下载结果是我主动发起的，不计入未读。

### 8.3 单实例

再次启动程序时：
1. 绑定 `47210` 失败；
2. 连接 `127.0.0.1:47210`，发送 `{"t":"show"}`；
3. 已运行的实例唤起主窗口，新进程退出。

如果第 2 步连接也失败，说明端口被其他程序占用，提示「端口 47210 被其他程序占用」。

### 8.4 开机自动启动（C2-6）

- 设置页提供开关「开机自动启动」，说明文字为「开启后，下次登录系统时 Local Echo 会自动在后台启动」。
- 开关**不另外存储状态**：打开设置页时直接读取系统中的自启项是否存在；切换开关时立即写入或删除自启项，下次登录系统时生效。
- 自启时带 `--hidden` 参数启动：只驻留托盘，不打开主窗口（已确认）。

| 平台 | 自启项 |
|---|---|
| Windows | 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `Local Echo`，数据为 `"<exe 路径>" --hidden`（用 `winreg` 读写） |
| Linux | `~/.config/autostart/local-echo.desktop`（XDG Autostart 规范，GNOME / KDE / Xfce 通用），内容见下 |

```ini
[Desktop Entry]
Type=Application
Name=Local Echo
Exec=/usr/bin/local-echo --hidden
TryExec=/usr/bin/local-echo
Icon=local-echo
Terminal=false
X-GNOME-Autostart-enabled=true
```

- `Exec` 和 `TryExec` 使用 `std::env::current_exe()` 的实际路径。`TryExec` 的作用是：软件卸载后，桌面环境会因为找不到程序而自动跳过这一项，避免留下失效的自启。
- Windows 卸载时，由安装程序删除注册表中的自启项（11.1）。

---

## 9. 界面结构（视觉由 Claude Design 产出）

```
┌───────────────────────────────────────────────────────────────┐
│ [logo] Local Echo                              — ☐ ✕ (标题栏) │
├──────────────────┬────────────────────────────────────────────┤
│ 联系人       [+] │  会话头：名称 · 地址        [会话操作按钮] │
│  老李 · .23  (2) │────────────────────────────────────────────│
│  小王 · .41      │              消息列表（虚拟滚动）           │
│ 服务器       [+] │                                            │
│  测试机          │                                            │
│  root@10.0.0.5   │                                            │
│──────────────────│────────────────────────────────────────────│ ← 同一像素行（底栏高 60）
│ [⚙ 设置]         │  底部：联系人 = 附件 / 输入框 / 发送       │
│                  │        服务器 = 拖拽区 + 上传按钮          │
└──────────────────┴────────────────────────────────────────────┘
```

| 视图 | 组成 | 主要组件 |
|---|---|---|
| 主窗口 | 自绘标题栏 + 侧边栏 + 会话区；默认 1000×680，最小 760×520 | `TitleBar`、`Sidebar`、`Avatar`、`Badge` |
| 联系人会话 | 头部（昵称 · IP）；消息列表；底部：附件按钮、多行输入框、发送按钮；拖入文件时显示覆盖层 | `MessageScroller`、`Message`、`Bubble`、`Attachment`、`Textarea` |
| 服务器会话 | 头部：名称、`user@host:port`、远程目录 +「更改」、「下载文件」「在终端中打开」「在远程桌面打开」（v0.2）；消息列表；底部：拖拽提示 +「上传文件」 | 同上 + `Button`、`Tooltip` |
| 添加 / 编辑联系人 | 添加：对方 IP、昵称；编辑：昵称、IP | `Dialog`、`Input` |
| 添加 / 编辑服务器 | 添加：名称、地址、用户名、密码，提交后显示步骤进度；编辑：名称、地址、用户名、远程桌面端口（v0.2，选填） | `Dialog`、`Input`、`Spinner` |
| 重新授权 | 名称、地址、用户名只读，只填密码 | `Dialog` |
| 远程浏览器 | 路径栏、上一级、文件列表、底部按钮随模式变化 | `Dialog`、`Table` |
| 同名文件确认 | 两种文案（第 6 节） | `Dialog` |
| 设置 | 本机 IP（v0.2，可收起的表格）、数据目录、开机自动启动、退出、版本；迁移确认与进度 | `Setting`、`Switch`、`Progress`、`Table` |
| 未找到远程桌面客户端（v0.2，仅 Linux） | 说明 + 三组可复制的安装命令 +「知道了」（5.7） | `Dialog` |
| 右键菜单 | 联系人：编辑、删除；服务器：编辑、删除 | 上下文菜单 |
| 托盘 | 常规 / 未读两种图标；菜单：打开 Local Echo、退出 | 系统原生 |

**v0.2 布局规则**（视觉见 `docs/ui/Local Echo v0.2.dc.html`）：

- **底栏高度 token = 60**（`kit::FOOTER_H`，不含 1 px 上边框）：侧边栏「设置」区（「设置」行 36 垂直居中）、联系人输入区（单行时 12 + 36 + 12；多行时向上增高，此时不要求对齐）、服务器底栏（拖拽区和「上传文件」高 36）都用它，三处上边框落在同一像素行。gpui-kit 的 rem 是 14 px（主题字号），`py_3` 只有 10.5 px，所以这三处的上下内边距写成像素 12。
- **服务器头部按钮**：会话区宽度 ≥ 720 时三个按钮都带文字；< 720（窗口 < 980）时「在终端中打开」「在远程桌面打开」收成 32 × 32 图标按钮加 Tooltip，「下载文件」保留文字。「在远程桌面打开」图标用 Lucide `monitor`，Tooltip「用远程桌面连接 {host}:{port}」；检测中显示 Spinner +「正在连接…」，整体 70% 不透明，保持原宽（最小 146），图标按钮的 Tooltip 改为「正在连接…」。

---

## 10. 工程结构

```
local-echo/
├── Cargo.toml
├── build.rs                         # 目标为 Windows 时嵌入 assets/app.ico（按 CARGO_CFG_TARGET_OS 判断，支持交叉编译）
├── assets/                          # Claude Design 产出
│   ├── app.ico
│   ├── icons/hicolor/…              # Linux 应用图标：48/128/256/512 PNG + scalable SVG
│   ├── tray.png / tray-unread.png   # 托盘图标，32×32
│   ├── theme.json                   # gpui-kit 主题（浅色 + 深色）
│   └── local-echo.desktop
├── packaging/windows/local-echo.iss
├── packaging/windows/cross/         # v0.2：交叉编译用的预编译 gpui 着色器及其生成脚本
├── scripts/build-windows.sh         # v0.2：在 Linux 上交叉编译 Windows 测试版
├── src/
│   ├── main.rs                      # 单实例 → 读指针文件 → 打开数据库 → 启动 tokio 与监听 → 托盘 → 主窗口（--hidden 时不开） → 事件桥
│   ├── data.rs                      # 数据目录、指针文件、device_id、迁移、文件名处理
│   ├── db.rs                        # 表结构与增删改查（含同名查询）
│   ├── peer.rs                      # 与人交互：协议、监听、发送
│   ├── host.rs                      # 与机器交互：认证与部署公钥、SFTP、打开终端
│   ├── rdp.rs                       # v0.2：在远程桌面打开（端口探测、.rdp 文件 / Linux 客户端探测）
│   ├── net.rs                       # v0.2：本机 IP 列表（过滤、排序）
│   ├── desktop.rs                   # 托盘（常规 / 未读）、开机自启、打开文件 / 文件夹
│   └── ui/
│       ├── app.rs                   # 主窗口、侧边栏
│       ├── chat.rs                  # 会话视图（按 kind 切换头部和底部）
│       ├── dialogs.rs               # 添加 / 编辑联系人、添加 / 编辑 / 重新授权服务器、远程浏览器、同名确认、未找到远程桌面客户端
│       ├── kit.rs                   # 设计规格中反复出现的视觉单元（底栏高度、表格、对话框外壳等）
│       └── settings.rs
└── .github/workflows/release.yml
```

```toml
[package]
name = "local-echo"
version = "0.1.0"
edition = "2024"
description = "Local Echo: LAN peer-to-peer chat, file transfer and SSH file exchange"
license = "Proprietary"

[dependencies]
gpui-kit = "=0.9.1"
tokio = { version = "1", features = ["rt-multi-thread", "net", "io-util", "fs", "sync", "time", "macros"] }
russh = { version = "0.65", default-features = false, features = ["ring", "rsa", "flate2"] }
russh-sftp = "3"
rand = "0.10"
rusqlite = { version = "0.41", features = ["bundled"] }
tray-icon = { version = "0.26", default-features = false, features = ["ksni"] }
image = { version = "0.25", default-features = false, features = ["png"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
dirs = "6"
local-ip-address = "0.6"
chrono = "0.4"
anyhow = "1"

[target.'cfg(windows)'.dependencies]
winreg = "0.56"

[target.'cfg(windows)'.build-dependencies]
winresource = "0.1"

[package.metadata.deb]
maintainer = "Local Echo"
section = "net"
depends = "$auto, libvulkan1, libwayland-client0, libfontconfig1, openssh-client"
assets = [
  ["target/release/local-echo", "usr/bin/", "755"],
  ["assets/local-echo.desktop", "usr/share/applications/", "644"],
  ["assets/icons/hicolor/256x256/apps/local-echo.png", "usr/share/icons/hicolor/256x256/apps/", "644"],
  # …其余尺寸同理
]

[package.metadata.generate-rpm]
assets = [
  { source = "target/release/local-echo", dest = "/usr/bin/local-echo", mode = "755" },
  { source = "assets/local-echo.desktop", dest = "/usr/share/applications/local-echo.desktop", mode = "644" },
  { source = "assets/icons/hicolor/256x256/apps/local-echo.png", dest = "/usr/share/icons/hicolor/256x256/apps/local-echo.png", mode = "644" },
  # …其余尺寸同理
]
[package.metadata.generate-rpm.requires]
openssh-clients = "*"
vulkan-loader = "*"
libwayland-client = "*"
fontconfig = "*"
```

`assets/local-echo.desktop`（已通过 `desktop-file-validate` 校验）：

```ini
[Desktop Entry]
Type=Application
Name=Local Echo
Comment=LAN peer-to-peer chat, file transfer and SSH file exchange
Exec=local-echo
Icon=local-echo
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=local-echo
```

`main.rs` 顶部加上 `#![cfg_attr(windows, windows_subsystem = "windows")]`，Windows 下运行时不弹出控制台窗口。

---

## 11. 构建、安装包与发布

### 11.1 安装包（已确认）

| 平台 | 形式 | 要点 |
|---|---|---|
| Windows | Inno Setup 安装程序 `local-echo-<版本>-windows-x86_64-setup.exe` | 安装到 `Program Files\Local Echo`，需要管理员权限；创建开始菜单快捷方式，桌面快捷方式可选；自动添加 TCP 47210 入站防火墙规则；升级或卸载前强制关闭正在运行的程序；卸载时删除防火墙规则和当前用户的开机自启项，保留用户数据；安装完成后以当前用户身份启动程序；界面为简体中文；不做代码签名 |
| Linux | `local-echo_<版本>-1_amd64.deb`、`local-echo-<版本>-1.x86_64.rpm` | 在 `ubuntu-22.04` 上构建，要求 glibc ≥ 2.35；依赖由包管理器自动安装 |

`packaging/windows/local-echo.iss`（以 UTF-8 BOM 编码保存）：

```iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{140AB8F3-F901-46F3-AA78-68A210345FB5}
AppName=Local Echo
AppVersion={#AppVersion}
AppPublisher=Local Echo
DefaultDirName={autopf}\Local Echo
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
SetupIconFile=..\..\assets\app.ico
UninstallDisplayIcon={app}\local-echo.exe
CloseApplications=force
RestartApplications=no
OutputDir=..\..\target\installer
OutputBaseFilename=local-echo-{#AppVersion}-windows-x86_64-setup

[Languages]
; ChineseSimplified.isl 取自 jrsoftware/issrc 标签 is-6_7_1（Files/Languages/Unofficial），
; windows-latest 预装的 Inno Setup 6.7.1 不自带简体中文。
Name: "chs"; MessagesFile: "ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"

[Files]
Source: "..\..\target\release\local-echo.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Local Echo"; Filename: "{app}\local-echo.exe"
Name: "{autodesktop}\Local Echo"; Filename: "{app}\local-echo.exe"; Tasks: desktopicon

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Local Echo"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Local Echo"" dir=in action=allow protocol=TCP localport=47210 program=""{app}\local-echo.exe"" profile=any"; Flags: runhidden
Filename: "{app}\local-echo.exe"; Description: "启动 Local Echo"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Local Echo"""; Flags: runhidden; RunOnceId: "DelFwRule"
Filename: "{sys}\reg.exe"; Parameters: "delete ""HKCU\Software\Microsoft\Windows\CurrentVersion\Run"" /v ""Local Echo"" /f"; Flags: runhidden; RunOnceId: "DelAutostart"
```

- 卸载时用 `reg delete` 清除开机自启项。Inno 的 `[Registry]` 段需要在安装时就创建这个值，而自启项是用户在设置页里手动开启的，所以不适用。
- `CloseApplications=force`：程序驻留托盘时可能没有可见窗口，Restart Manager 无法请求它正常关闭。

### 11.2 GitHub Actions

```yaml
# .github/workflows/release.yml
name: release
on:
  push:
    tags: ["v*"]
permissions:
  contents: write

jobs:
  windows:
    runs-on: windows-latest                  # 预装 Inno Setup 6.7.1
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release --locked
      - name: Build installer
        shell: pwsh
        run: |
          $v = "${{ github.ref_name }}".TrimStart("v")
          & "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" "/DAppVersion=$v" packaging\windows\local-echo.iss
      - uses: actions/upload-artifact@v4
        with: { name: windows, path: target/installer/*.exe }

  linux:
    runs-on: ubuntu-22.04                    # glibc 2.35
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc g++ pkg-config libfontconfig-dev libwayland-dev \
            libxkbcommon-x11-dev libx11-xcb-dev libvulkan1
          cargo install --locked cargo-deb cargo-generate-rpm
      - run: cargo build --release --locked
      - name: Build deb / rpm
        run: |
          cargo deb --no-build
          cargo generate-rpm
      - uses: actions/upload-artifact@v4
        with:
          name: linux
          path: |
            target/debian/*.deb
            target/generate-rpm/*.rpm

  release:
    needs: [windows, linux]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with: { path: dist, merge-multiple: true }
      - uses: softprops/action-gh-release@v2
        with: { files: dist/* }
```

发版流程：修改 `Cargo.toml` 中的版本号 → 提交 → `git tag v0.1.0 && git push --tags`。

---

## 12. 技术验证记录（2026-11-09，Ubuntu 26.04 / Rust 1.99.1）

| # | 验证项 | 结果 |
|---|---|---|
| 1 | gpui-kit 仓库与版本 | ✅ 原 gpui-component 已改名为 gpui-kit；`gpui-kit 0.9.1` 固定 `gpui-pre =0.3.12` |
| 2 | gpui-kit 在 Linux 上编译运行 | ✅ |
| 3 | 聊天组件 | ✅ `Message`、`Bubble`、内联图片、`Attachment` 进度卡片，截图确认 |
| 4 | 剪贴板图片 / 文件 | ✅ 源码确认 Windows、Wayland、X11 都支持；可通过 `Textarea::on_paste` 拦截 |
| 5 | 拖放、文件对话框、打开文件、在文件夹中显示、重启 | ✅ 源码确认 |
| 6 | 关闭全部窗口后进程不退出，并能重建窗口 | ✅ `QuitMode::Explicit` |
| 7 | tokio 通道 → gpui 前台的事件桥 | ✅ |
| 8 | Linux 托盘 | ✅ `tray-icon`（ksni）在 GNOME（AppIndicator 扩展）上注册成功，无 GTK 依赖；源码确认 KSNI 后端支持运行时 `set_icon` / `set_tooltip` |
| 9 | 没有托盘宿主时 | ⚠️ `build()` 仍然返回成功，无法探测；按已确认的方案不做探测 |
| 10 | russh 密码认证、错误密码识别、用 SFTP 部署公钥、密钥登录 | ✅ |
| 11 | 系统 `ssh -i` 用部署的密钥登录 | ✅ |
| 12 | SFTP 上传 / 下载 64 MB 并校验 | ✅ 本机回环 128–244 MB/s |
| 13 | russh 使用 `ring` 后端 | ✅ 依赖树中不再有 aws-lc |
| 14 | cargo-deb | ✅ 能自动推导依赖；在 26.04 上构建会要求 `libc6 >= 2.43`，说明必须在 22.04 上构建 |
| 15 | cargo-generate-rpm | ✅ 能自动推导依赖，手动列出的依赖也生效 |
| 16 | `.desktop` 与开机自启的 `.desktop` | ✅ 都通过 `desktop-file-validate` |
| 17 | Inno Setup | ✅ `windows-latest` 预装 6.7.1，但**不带** `ChineseSimplified.isl`（首次 CI 构建失败才发现；6.7.1 时它还在 Unofficial 目录），改为把该版本的语言文件放进 `packaging/windows/`；`[Registry]` 段的 `uninsdeletevalue` 不适用于「运行时才创建的值」，所以改用 `reg delete`（依据官方文档） |
| 18 | chrono 是否已在依赖树中 | ✅ `chrono 0.4.45`（含 `clock` 特性）已由 gpui-base 引入 |
| 19 | 第一版实现端到端走查（2026-11-10，两个实例分处不同网络命名空间，运行在隔离的 Xvfb 显示上） | ✅ 添加 / 编辑联系人、被动建立联系人、双向文本、粘贴图片（两端各自按时间戳命名）、附件发送文件（内容一致）、同名文件确认（联系人与服务器两种）、设备不匹配拒收与重试、添加服务器（错误密码 / 四步进度）、上传 / 下载、更改远程目录、在终端中打开（xterm）、重新授权、关窗驻留与单实例唤起、关窗期间的未读计数、开机自启开关、跨文件系统的数据迁移与自动重启、删除服务器（保留文件） |
| 20 | 按 Claude Design 交付稿（docs/ui）实现界面（2026-11-10） | ✅ 浅色 / 深色主题、侧边栏（多 IP 网卡名、空分组引导、52 px 双行条目、方形未读徽标）、消息分组与日期分隔、气泡 / 发送状态 / 设备不匹配提示、图片等比尺寸、文件卡片（中间省略、进度、操作行、失败重试）、服务器页与密钥失效横幅、各对话框（步骤进度、只读字段、浏览器表格、同名列表、删除 / 迁移确认）、设置页均在隔离 Xvfb 中截图核对。交付主题 JSON 中 7 个键名与 gpui-kit 0.9.1 不一致（如 `link.foreground` 应为 `link`）已更正，并补充实心危险按钮配色 |

| 21 | v0.2 单元测试（2026-11-11，Linux） | ✅ `resolve_stored` 逐段拼接、v0.1 表结构升级到 `user_version = 1`（含重复打开与 v0.1 按列名读写升级后的库）、`.rdp` 内容（UTF-16LE + BOM）、URI（IPv6、特殊字符用户名）、Linux 探测顺序（`which` 作为参数传入）、远程桌面端口校验、端口探测、本机 IP 过滤与排序 |
| 22 | v0.2 界面（2026-11-11，隔离的 Xvfb，浅色 / 深色，窗口 1000 × 680 与 760 × 520） | ✅ 截图逐项核对：侧边栏从「联系人」开始；文件卡片四种「已完成」与「文件已被移动或删除」；删除文件后点击「打开」立即提示并切换状态；三处底部分割线同在第 619 行（输入区内容 632–667，即 12 + 36 + 12）；服务器头部三个文字按钮 / 窄宽度下的图标按钮与 Tooltip、「正在连接…」、与密钥失效横幅同时出现、端口不通提示、「未找到远程桌面客户端」对话框与复制；编辑服务器的端口字段（只能输入数字、70000 报错、保存 3390 生效）；设置页本机 IP 展开 / 收起 / 复制 / 双击复制 / 无网络 / 最小窗口。测试实例用 bwrap 隔离网络，或用 `LD_PRELOAD` 把 47210 改到别的端口，不影响本机正在运行的 Local Echo |
| 23 | Linux 交叉编译 Windows 版（2026-11-11，`scripts/build-windows.sh`） | ✅ `local-echo.exe` 为 GUI 子系统、只依赖系统 DLL、带图标资源；单元测试程序可交叉编译 |
| 24 | Windows 测试机验收（2026-11-11/12，Windows 11 家庭版 25H2，150% 缩放，交叉编译的测试版 + 用户数据副本，数据目录放在带空格和中文的路径 `le-test\数据 目录`） | ✅ 单元测试 23 项全部通过（含 Windows 专用的「路径不含 `/`」断言）；v0.1 数据库副本自动升级；左右底部分割线同在物理像素第 929 行；本机 IP 表格（WLAN 在前、Tailscale 在后，3 个 169.254 地址被过滤）；收到 / 下载 / 上传三种文件卡片的「打开文件夹」都打开资源管理器并选中文件，目录已有窗口时也能选中；外部删除文件后切回窗口，卡片换成「文件已被移动或删除」，图片退化为文件卡片；原来被改名的两个文件（#13 的原始场景）显示提示；添加服务器（Docker 中的 sshd）、下载、上传正常；「在远程桌面打开」拉起 mstsc，`.rdp` 为 UTF-16LE + BOM，经 2026 年 4 月新增的安全警告和证书警告后连上 Docker 中的 xrdp，登录框预填用户名，进入会话；中文用户名正确传递（2 个字符） |

| 25 | 远程桌面 Linux 客户端实测（2026-11-12，Docker：一个 Ubuntu 26.04 容器运行 xrdp + sshd，另一个装 Remmina 1.4.43、KRDC、FreeRDP 3 SDL、GNOME Connections 并在 Xvfb 中运行 Local Echo） | ✅ 依次停用排在前面的客户端，Local Echo 分别拉起 `remmina -c rdp://echotest@le-rdp-server:3389`（认证框预填用户名）、`krdc rdp://echotest@…`（首次弹主机配置，密码框预填用户名）、`sdl-freerdp3 /v:… /u:echotest /t:xrdp 测试机 …`（SDL 凭据框预填用户名，窗口标题为服务器名称），都输入密码后登录进 xrdp 会话（以 xrdp-sesman 日志为准）；都不可用时用 `xdg-open` 把 `rdp://echotest@le-rdp-server:3389` 交给系统默认处理程序。GNOME Connections 从命令行收到 `rdp://le-rdp-server:3389` 后直接连接，验证指纹、输入凭据后登录成功，因此加入探测列表，并端到端确认它排在默认处理程序之前 |

**未验证**：
- v0.2：Flatpak 版 Remmina（只由单元测试覆盖探测与拉起命令）；mstsc 连接 Windows 远程桌面（需要专业版或服务器版 Windows 作被连接端，含「记住我」）。
- Windows 端：编译、托盘、剪贴板、`ssh.exe` 对私钥 ACL 的检查、拉起终端、注册表自启、Inno 编译与卸载清理、防火墙规则。
- 在 ubuntu-22.04 上编译。
- 在干净的 Fedora / Debian 上安装。
- 真实局域网环境下的吞吐、中文输入法。

---

## 13. 风险

| # | 风险 | 对策 |
|---|---|---|
| R1 | 服务器禁用了公钥认证，或 home 目录只读 | 明确报错（已确认方案的固有限制） |
| R2 | 数据目录迁移到 Windows 非用户目录后 ACL 过宽，`ssh.exe` 拒绝使用私钥 | 迁移后执行 `icacls` |
| R3 | Linux 终端种类繁多 | 探测表 + 明确提示 |
| R4 | Linux 开启防火墙时 47210 被拦截（Ubuntu 的 ufw 默认关闭；Fedora Workstation 默认放行 1025–65535） | 在 README 中说明 |
| R5 | 原生 GNOME 没有托盘宿主，看不到托盘图标 | 已确认：程序照常驻留后台，再次启动可唤起窗口，设置页可退出 |
| R6 | 安装程序未签名，Windows 会弹出 SmartScreen 提示 | 已确认接受 |
| R7 | 卸载程序以管理员身份运行；如果管理员账户和日常用户不是同一个，`HKCU` 指向管理员账户，日常用户的自启项不会被清除 | 残留的自启项指向已删除的 exe，登录时静默失败，没有其他影响；常见情况（日常用户本身就是管理员）下不会发生 |
| R8 | gpui-kit / GPUI 迭代快 | 固定 `=0.9.1` |
| R9 | Linux X11 会话下，粘贴「已复制的文件」不会触发发送：gpui 的 X11 剪贴板没有实现 `text/uri-list`（实现阶段发现） | 拖拽和附件按钮不受影响；Wayland、Windows 正常。已写入 README |
| R10 | 设计稿使用 600 字重；系统字体缺少半粗体时（如部分 Linux 的 Noto Sans 只有常规 / 粗体）会显示为常规字重 | Windows（Segoe UI）与带 Ubuntu Sans 的系统正常；不内嵌字体（设计稿约定使用系统字体） |
| R11 | 服务器没有开启 RDP 服务（v0.2） | 5.7 的端口探测给出明确提示；README 中说明服务器端的前提 |
| R12 | GNOME 远程桌面使用单独设置的 RDP 账号，与系统账号不同，预填的 SSH 用户名可能不对 | 用户在客户端的登录框里修改；写入 README |
| R13 | xrdp 的已知限制：同一个用户已经在服务器本地登录了图形会话时，xrdp 连接会黑屏 | 写入 README，不在本工具中处理 |
| R14 | GNOME Connections 从命令行传入 `rdp://` 的行为未经实测 | 已解决：2026-11-11 在 Docker 中实测能直接连接，加入探测列表（5.7） |

---

## 14. 确认记录与新增待确认事项

### 14.1 第一轮确认（2026-11-09）

| # | 事项 | 结论 |
|---|---|---|
| 1 | 数据目录合一 | 按建议 |
| 2 | 密码只用一次，用于部署公钥 | 按建议 |
| 3 | 拉起系统终端 | 按建议 |
| 4 | 局域网明文传输 | 按建议 |
| 5 | 不校验主机指纹 | 按建议 |
| 6 | 自动接收；不做取消、断点续传、文件夹传输 | 按建议 |
| 7 | 发出的文件不复制，图片复制 | 按建议 |
| 8 | 对端身份 | 添加时设置昵称；可编辑、可修改 IP；不以 IP 作为唯一标识 |
| 9 | 通知 / 托盘 | 不要系统通知，要托盘常驻 |
| 10 | 固定端口 47210 | 按建议 |
| 11 | 同名上传 | 弹窗告知上次上传时间，确认后继续 |
| 12 | 表情 | 移除 |
| 13 | 发布形态 | 两个平台都做安装包 |
| 14 | 中文名 | 不需要 |
| 15 | 删除联系人 | 删除联系人和聊天记录，保留文件 |
| 16 | 支持范围 | 按建议 |

### 14.2 第二轮确认（2026-11-09）

| # | 事项 | 结论 | 落点 |
|---|---|---|---|
| 1 | 对端 IP 自动更新 | 按建议 | 4.1 |
| 2 | 同名确认范围 | 也包括给联系人发文件；「同事」统一改为「联系人」 | 6 |
| 3 | 同名判断依据 | 只依据本工具的记录 | 6 |
| 4 | 托盘未读提示 | 按建议 | 8.2 |
| 5 | 没有托盘宿主的 Linux 桌面 | 按建议 | 8.1 |
| 6 | 开机自启 | 做，开关放在设置页，下次启动生效 | 8.4 |
| 7 | 安装包细节 | 按建议 | 11.1 |
| 8 | 服务器名称与编辑 | 增加名称字段和编辑功能 | 5.3 |

### 14.3 第三轮确认（2026-11-09）

| # | 事项 | 结论 | 落点 |
|---|---|---|---|
| 1 | 同名确认是否包括图片 | 只针对文件；图片保存时按时间戳重命名 | 6、3.3 |
| 2 | 开机自启时不弹出主窗口 | 按建议 | 8.4 |
| 3 | 编辑服务器规则 | 按建议 | 5.3、3.2 |

### 14.4 第四轮确认（2026-11-09）

| # | 事项 | 结论 | 落点 |
|---|---|---|---|
| 1 | 图片时间戳命名的细节：本地时间 `YYYYMMDD-HHMMSS-mmm.<扩展名>`，收发两端各自命名，不保留原文件名 | 按建议 | 3.3 |

至此没有待确认事项，方案定稿。

### 14.5 第五轮确认（2026-11-11，v0.2）

| # | 事项 | 结论 | 落点 |
|---|---|---|---|
| 1 | 设置页「本机 IP」默认展开还是收起 | 默认展开；本次运行期间记住用户的选择，不写盘 | 7.3 |
| 2 | 服务器底栏是否一并改为 60 | 一起改 | 9 |
| 3 | 远程桌面端口是否可配置 | 只在「编辑服务器」中提供，默认 3389；添加服务器流程不变 | 5.7 |
| 4 | 远程桌面用户名是否默认使用 SSH 用户名 | 是，可在客户端登录框里修改；不另设字段 | 5.7 |
| 5 | Linux 找不到客户端时 | 只提示安装命令，不退回到终端里运行 `xfreerdp` | 5.7 |
| 6 | deb / rpm 是否声明 Suggests | 是 | 5.7 |
| 7 | B1 是否先单独发布 v0.1.1 | 不单独发布，全部完成后统一发布 v0.2.0 | — |
| 8 | 本机 IP 是否过滤 169.254.x.x | 过滤 | 7.3 |
| 9、10 | 发布流程运行单元测试 / 手动触发测试安装包 | 撤回：改用 Windows 测试机，`release.yml` 不改动 | — |
| 11 | 测试机上做哪些准备 | 测试机不装开发环境，在 Linux 本机交叉编译后拿到测试机上测 | 10、12 |
| 12 | 图形界面验收谁来操作 | 我通过计划任务在测试机桌面会话中操作，每次开始前先征得同意 | — |
| 13 | 文件被移动、改名或删除后卡片是否提示 | 在操作行的位置显示「文件已被移动或删除」，不提供「重新定位文件」 | 4.4 |

---

## 附录 A：Claude Design 提示词

> 用法：把下面代码块中的全部内容粘贴给 Claude Design。产出物放到仓库的 `assets/`（图标、主题）和 `docs/ui/`（设计稿）。

````markdown
# 角色与任务

你是一名资深桌面应用品牌与 UI/UX 设计师。请为桌面工具 **Local Echo** 完成两部分设计：
1. 品牌 Logo、应用图标、托盘图标；
2. 完整的 UI/UX 高保真设计（浅色、深色两套主题），以及可直接落地的设计规范和主题色板。

设计将由 Rust + GPUI（gpui-kit 组件库）原样实现，请严格遵守第六节「实现约束」。

---

# 一、产品概述

Local Echo 是一款**极致轻量**的局域网点对点工具，用「聊天」的形式完成两件事：

1. **与人交互**：和局域网内的**联系人**互发文本、图片、文件。通过对方 IP 添加，并为对方设置昵称；双方都安装 Local Echo，消息直接点对点传输，**不经过任何服务器或公网**。
2. **与机器交互**：把 Linux 服务器也当作一个「聊天对象」。添加时填写名称、IP、用户名、密码，之后：
   - 向服务器上传文件，相当于我给它「发消息」（显示在右侧）；
   - 从服务器下载文件，相当于它给我「发消息」（显示在左侧）；
   - 一键「在终端中打开」，免密进入这台服务器的当前目录。

程序关闭窗口后**常驻系统托盘**继续接收，并支持开机自动启动。

- 目标用户：经常在局域网内传文件、经常登录服务器的工程师和办公人员。
- 核心价值：**不依赖公网，不用记 IP、账号和密码，打开就能用。**
- 平台：Windows 10/11、Linux（GNOME / KDE / Xfce）。
- 界面语言：简体中文。产品名只用英文「Local Echo」。

---

# 二、品牌与 Logo

## 品牌关键词
轻、快、直连、可靠、安静、工程感。「Echo」即回声：信息在两点之间直接往返，没有中间人。

## Logo 方向（至少 3 个，每个附一句设计说明，并给出推荐）
- 可考虑的意象：两点直连、声波或回声的同心弧、双向往返、对话气泡与连线的结合。
- 避免：云朵（本产品恰恰不用云）、地球、复杂渐变或立体效果、过于卡通。
- 要求：16×16 下仍可辨认；有单色版本；在浅色和深色背景上都清晰。

## 交付的品牌物料
1. Logo 主图形（SVG 源文件），以及横版组合（图形 + 「Local Echo」字标）。
2. 应用图标 PNG：16、24、32、48、64、128、256、512 px。
3. Windows `.ico`：包含 16、24、32、48、64、256 多个尺寸。
4. Linux 应用图标：48、128、256、512 px PNG，以及 scalable SVG。
5. **托盘图标**两种状态：常规；有未读消息（右上角加小圆点）。每种都提供 16、20、24、32 px 的 PNG，在 Windows 浅色 / 深色任务栏和 Linux 深色顶栏上都要清晰。
6. 标题栏小图标：16 px、20 px。
7. 品牌主色和辅助色说明（与第八节的主题色板一致）。

---

# 三、视觉风格

- 极简、克制，信息密度适中。整体接近 shadcn/ui 的现代桌面风格：留白充足，细边框，小圆角，几乎不用阴影。
- **只用 1 个品牌强调色**，用于主按钮、我方消息气泡、选中态和进度条；其余使用中性灰阶。
- 浅色和深色两套主题都要完整设计，跟随系统切换。
- 字体用系统 UI 字体（Windows：Segoe UI / 微软雅黑；Linux：系统默认无衬线字体），优先保证中文显示效果。字号层级不超过 5 级。
- 图标**只使用 Lucide 图标集**。
- 对比度满足 WCAG AA。

---

# 四、信息架构与界面清单

窗口默认 1000×680，最小 760×520，使用**自绘标题栏**（Logo、应用名、窗口控制按钮）。
布局：**左侧边栏约 260 px + 右侧会话区**。请逐屏设计，并覆盖所列的全部状态。

## 1. 主窗口与侧边栏
- 顶部：「本机 IP：192.168.1.10」，点击复制并给出轻提示；有多个 IP 时全部列出。
- 分组「联系人」，组标题右侧有「+」（添加联系人）。每个条目：首字母头像（取昵称首字）、**昵称**、IP（次要文字）、未读数徽标。
- 分组「服务器」，组标题右侧有「+」（添加服务器）。每个条目：服务器图标（Lucide `server`）、**服务器名称**、`user@host`（次要文字，端口不是 22 时才显示端口）。
- 底部：「设置」入口。
- 状态：选中、悬停、有未读。
- 右键菜单：联系人与服务器都有「编辑」「删除」。删除需二次确认。
- 空状态：两个分组都为空时，引导用户添加。

## 2. 未选中会话
Logo、一句话说明，以及「添加联系人」「添加服务器」两个按钮。

## 3. 联系人会话
- 头部：头像、昵称、IP。
- 消息列表：对方消息在左（带头像），我的消息在右（品牌色气泡）；显示时间，跨天时插入日期分隔。
- **每种消息类型和状态都要画出来**：
  - 文本：短文本、多行长文本；
  - 图片：内联缩略图，最大 240×240、保持比例；横图、竖图、小图各一例；
  - 文件卡片：文件类型图标、过长的文件名中间省略、大小、进度条与百分比。状态包括：传输中；已完成（「打开」「打开所在文件夹」）；失败（「发送失败 · 重试」）；
  - 文本的发送中、发送失败（带重试）；
  - 设备不匹配导致的失败：「发送失败：192.168.1.23 当前不是「老李」的设备，请修改 IP」，并提供「编辑」入口。
- 底部输入区：附件按钮、多行输入框（自动增高，最多约 6 行）、发送按钮；占位文字「输入消息，Enter 发送，Shift+Enter 换行」。**没有表情按钮。**
- 拖拽覆盖层：「松开以发送给 老李」。

## 4. 服务器会话
- 头部：服务器图标、**服务器名称**（主标题）、`root@10.0.0.5:22`（副标题）；第二行为「远程目录：/home/root/deploy」+「更改」；右侧按钮：「下载文件」「在终端中打开」。
- 消息列表：上传是我发出的文件卡片（右侧）；下载是服务器发来的文件卡片（左侧，头像位置用服务器图标）。卡片额外显示远端路径。
- 底部（没有文字输入框）：虚线拖拽区「拖拽文件到此处，上传到 /home/root/deploy」，右侧「上传文件」按钮。
- 拖拽覆盖层：「松开以上传到 /home/root/deploy」。
- 密钥失效横幅：「无法登录该服务器，访问密钥可能已失效」+「重新授权」。

## 5. 同名文件确认（对话框，两种文案，同一个版式；只针对文件，发图片不会弹出）
- 联系人：标题「已发送过同名文件」；每行「文件名 — 上次发送于 2026-11-08 14:32」；说明「继续发送后，对方会再收到一份。」；按钮「取消」「继续发送」。
- 服务器：标题「已上传过同名文件」；每行「文件名 — 上次上传于 2026-11-08 14:32」；说明「继续上传将覆盖服务器上 /home/root/deploy 中的同名文件。」；按钮「取消」「继续上传」。
- 列表需要能容纳多行（一次拖入多个文件）。

## 6. 远程目录浏览器（对话框，约 640×480）
- 顶部：「上一级」按钮 + 可编辑的路径栏。
- 列表：图标、名称、大小、修改时间；文件夹排在前面；双击进入。
- 两种模式：
  - 选择目录：底部「取消」「选择此目录」；
  - 选择文件：可多选，底部「已选 3 项」+「取消」「下载所选」。
- 状态：加载中、空目录、无法读取。

## 7. 添加联系人 / 编辑联系人（对话框）
- 添加：字段「对方 IP」「昵称」（均必填）；提示「对方需已运行 Local Echo」；按钮「取消」「添加」；状态：连接中、连接失败。
- 编辑：字段「昵称」「IP」；提示「修改 IP 后，消息将发送到新地址」；按钮「取消」「保存」。

## 8. 添加服务器 / 编辑服务器 / 重新授权（对话框）
- 添加：字段「名称」（占位「如：测试服务器」）、「地址」（占位 `192.168.1.20 或 192.168.1.20:2222`）、「用户名」、「密码」，均必填。
  - 说明（醒目但克制）：「密码仅用于本次登录并部署访问密钥，不会被保存。之后上传、下载、打开终端都无需再输入密码。」
  - 提交后显示步骤进度：① 连接服务器 → ② 验证密码 → ③ 部署访问密钥 → ④ 验证密钥登录；每步有进行中、完成、失败三种状态。
- 编辑：字段「名称」「地址」「用户名」（没有密码）；提示「修改地址或用户名后，如无法登录，可通过「重新授权」重新部署访问密钥」；按钮「取消」「保存」。
- 重新授权：名称、地址、用户名只读，只需填写密码；步骤进度同上。

## 9. 设置页（请给出建议：放在右侧会话区，还是用对话框）
- 「数据目录」：显示路径，「打开」「更改…」；说明「聊天记录、收到的文件和服务器访问密钥都保存在此目录」。
- 「开机自动启动」：开关；说明「开启后，下次登录系统时 Local Echo 会自动在后台启动」。
- 迁移确认（两种情况）：
  - 「使用该目录中的已有数据？当前数据将保留在原位置。」
  - 「将全部数据移动到 X？」
- 迁移进度：进度条 +「正在移动 1.2 GB / 3.4 GB」，完成后自动重启。迁移受阻时提示「有传输正在进行，请完成后再迁移」。
- 「退出 Local Echo」按钮：关闭窗口只会驻留托盘，这里才是彻底退出；使用次要或危险样式。
- 「关于」：版本号。

## 10. 托盘
- 托盘图标的常规与未读两种状态。
- 悬停提示：「Local Echo」/「Local Echo · 3 条未读」。
- 右键菜单（系统原生样式，只需确认文案）：「打开 Local Echo」「退出」。

## 11. 全局提示
- Toast：复制成功、文件已保存、「该联系人已在列表中，已更新 IP 和昵称」。
- 错误对话框：「端口 47210 被其他程序占用。」

---

# 五、交互规范

- Enter 发送，Shift+Enter 换行；在输入框粘贴图片直接发送；粘贴已复制的文件则按文件发送。
- 拖拽文件到会话区：联系人会话为发送，服务器会话为上传；可一次拖入多个文件，每个文件生成一条消息。
- 发送或上传**文件**前，如果检测到同名记录，先弹出同名文件确认；图片直接发送。
- 新消息到达时，若用户停留在底部则自动滚动；若正在查看历史，则显示「↓ 新消息」按钮。
- 所有耗时操作都要有明确的进行中状态，以及成功或失败反馈。
- 动效克制，只用于悬停、按下、浮层出现、进度变化，时长不超过 150 ms。

---

# 六、实现约束（必须遵守）

1. 只能使用 gpui-kit（shadcn 风格的 GPUI 组件库）已有的组件形态：
   TitleBar、Sidebar（组 / 菜单项 / 页脚）、Avatar、Badge、Button（primary / secondary / ghost / link / danger）、Input、Textarea、Switch、Dialog、Popover、Tooltip、Notification（Toast）、Alert、Progress、Spinner、Table、List、Breadcrumb、Setting（设置页分组）、
   **Message / MessageGroup / Bubble（聊天气泡）、Attachment（带进度条和完成 / 失败状态的文件卡片）、MessageScroller（虚拟化消息列表，自带「跳到最新」按钮）**。
2. 不使用 Web 特有效果：毛玻璃、复杂渐变网格、视差、Lottie、自定义 Web 字体。
3. 采用 4 px 栅格；只用纯色和 1 px 细边框；阴影只用于浮层。
4. 适配 1× 和 2× 缩放；中文文本可能比英文长 30%，需留出余量。
5. 头像使用首字母头像，不使用照片。
6. 托盘菜单是系统原生菜单，不能自定义样式。

---

# 七、界面文案（请直接使用，可以提出优化建议）

| 位置 | 文案 |
|---|---|
| 侧边栏分组 | 联系人 / 服务器 |
| 侧边栏顶部 | 本机 IP：{ip} |
| 添加联系人 | 标题「添加联系人」；字段「对方 IP」「昵称」；提示「对方需已运行 Local Echo」；按钮「添加」 |
| 添加联系人失败 | 无法连接 {ip}：对方未运行 Local Echo，或网络不可达 |
| 编辑联系人 | 标题「编辑联系人」；字段「昵称」「IP」；提示「修改 IP 后，消息将发送到新地址」；按钮「保存」 |
| 设备不匹配 | 发送失败：{ip} 当前不是「{昵称}」的设备，请修改 IP |
| 添加服务器 | 标题「添加服务器」；字段「名称」「地址」「用户名」「密码」；按钮「连接并添加」 |
| 编辑服务器 | 标题「编辑服务器」；字段「名称」「地址」「用户名」；按钮「保存」 |
| 添加服务器失败 | 无法连接 {host}:{port} / 用户名或密码错误 / 公钥已写入，但无法用密钥登录：服务器可能禁用了公钥认证 |
| 服务器会话 | 下载文件 / 在终端中打开 / 上传文件 / 更改 / 重新授权 |
| 同名确认（联系人） | 已发送过同名文件 / {文件名} — 上次发送于 {时间} / 继续发送后，对方会再收到一份。/ 取消 / 继续发送 |
| 同名确认（服务器） | 已上传过同名文件 / {文件名} — 上次上传于 {时间} / 继续上传将覆盖服务器上 {目录} 中的同名文件。/ 取消 / 继续上传 |
| 远程浏览器 | 上一级 / 选择此目录 / 下载所选 / 已选 {n} 项 / 空目录 / 无法读取该目录：{原因} |
| 文件卡片 | 打开 / 打开文件夹（v0.2 由「打开所在文件夹」改名） / 重试 / 发送失败 / 上传失败 / 下载失败 |
| 终端 | 未找到可用的终端程序 |
| 删除确认 | 删除「{名称}」？聊天记录将被删除，已传输的文件会保留。 |
| 设置 | 数据目录 / 打开 / 更改… / 开机自动启动 / 开启后，下次登录系统时 Local Echo 会自动在后台启动 / 退出 Local Echo / 关于 / 版本 {v} |
| 托盘菜单 | 打开 Local Echo / 退出 |
| 托盘悬停提示 | Local Echo / Local Echo · {n} 条未读 |

---

# 八、交付物

1. **品牌物料**：第二节全部内容（包括两种状态的托盘图标）。
2. **高保真设计稿**：第四节所有界面和状态，浅色、深色各一套，画布 1000×680（对话框按实际尺寸）。
3. **组件规格标注**：侧边栏条目、消息气泡、文件卡片、输入区、拖拽区、对话框的尺寸、内边距、圆角、字号和颜色 token。
4. **主题色板**：以 **gpui-kit 主题 JSON** 格式输出，包含 `"mode": "light"` 与 `"mode": "dark"` 两个主题，颜色写作 `#RRGGBB` 或 `#RRGGBBAA`，需覆盖以下全部键：
   `background, foreground, border, ring, caret, overlay, selection.background, window.border,
   primary.background, primary.foreground, primary.hover.background, primary.active.background,
   secondary.background, secondary.foreground, secondary.hover.background, secondary.active.background,
   muted.background, muted.foreground, accent.background, accent.foreground,
   danger.background, danger.foreground, success.background, success.foreground,
   warning.background, warning.foreground, info.background, info.foreground,
   input.border, popover.background, popover.foreground,
   list.background, list.hover.background, list.active.background, list.active.border, list.even.background, list.head.background,
   table.background, table.head.background, table.head.foreground, table.hover.background, table.active.background, table.active.border, table.even.background, table.row.border,
   sidebar.background, sidebar.foreground, sidebar.border, sidebar.accent.background, sidebar.accent.foreground, sidebar.primary.background, sidebar.primary.foreground,
   title_bar.background, title_bar.border, status_bar.background, status_bar.border,
   tab.background, tab.foreground, tab.active.background, tab.active.foreground, tab_bar.background, tab_bar.segmented.background,
   progress_bar.background, skeleton.background, slider.bar.background, slider.thumb.background, switch.background,
   scrollbar.background, scrollbar.thumb.background, scrollbar.thumb.hover.background,
   drag_border, drop_target.background, link.foreground, link.hover.foreground, link.active.foreground,
   group_box.background, group_box.foreground, accordion.background, description_list_label.foreground,
   chart.1, chart.2, chart.3, chart.4, chart.5, chart.grid, chart.bullish, chart.bearish,
   base.red, base.red.light, base.green, base.green.light, base.blue, base.blue.light, base.yellow, base.yellow.light, base.magenta, base.magenta.light, base.cyan, base.cyan.light`
   另外请注明我方气泡、对方气泡分别使用哪个 token（或新增的两个颜色值）。
5. **设计说明**：一页纸，说明 Logo 寓意、配色理由、关键交互取舍。

---

# 九、不需要设计的内容

表情面板、在线状态、群聊、已读回执、撤回或编辑消息、搜索、头像上传、系统通知、登录 / 注册、云同步、移动端、安装程序向导页面（使用 Inno Setup 默认样式）。
````

---

## 附录 B：Claude Design 提示词（v0.2 增量）

> 用于程序 v0.2.0 的 4 处界面修改，在附录 A 的第一版设计稿上增量修改。交付稿已覆盖到 `docs/ui/`（入口 `Local Echo v0.2.dc.html`）。确认记录见 14.5，详细方案见 [plan-v0.2.md](plan-v0.2.md)。

````markdown
# 角色与任务

你是 Local Echo 的 UI/UX 设计师。第一版设计（本项目中的 LE Window、Local Echo 界面 / 规范 / 品牌）已经交付，并已按原样用 Rust + GPUI（gpui-kit 组件库）实现。

这次是 **v0.2 增量修改**，只改下面列出的 4 处。其余界面、品牌、色板、组件规格一律保持不变，**不要重新设计**。请在原设计稿上修改，沿用原有的颜色 token、4 px 栅格、字号层级，以及浅色 / 深色两套主题。

产品简述：Local Echo 是局域网点对点工具，用「聊天」的形式和联系人互发文本、图片、文件，并把 Linux 服务器当作「聊天对象」，通过 SSH 上传和下载文件、在终端中打开。窗口默认 1000×680，最小 760×520；左侧边栏 260 px，右侧是会话区。

---

# 修改 1：文件卡片操作行（含「文件已不存在」状态）

**现状**：文件传输完成后，卡片底部有一行操作「打开」「打开所在文件夹」：两个文字按钮按内容宽度靠左排列，高 32，中间有 1 px 竖线，链接色，13 px / 600。

**要求**：
- 两个按钮**均分卡片整宽**（各占 50%），文字水平居中；1 px 竖线、高度 32、链接色、字号不变。
- 「打开所在文件夹」改名为 **「打开文件夹」**。
- 悬停：所在的那一半整块显示 muted 背景。
- 联系人卡片宽 320，服务器卡片宽 340（服务器卡片多一行远端路径），两种宽度都要更新。我发出的、对方发来的、上传、下载四种卡片的「已完成」状态都要更新。
- **新增状态「文件已不存在」**：传输成功，但本地文件后来被移动、改名或删除。现在这种情况下操作行被直接隐藏，用户看不出原因。改为在操作行的位置显示一行提示「文件已被移动或删除」：
  - muted 色，13 px，左对齐，内边距与操作行一致；
  - 高 32，保留上边框，与操作行等高，卡片高度不变；
  - 不可点击，不提供「重新定位文件」之类的操作。
  请画出联系人卡片和服务器卡片各一例。

---

# 修改 2：「本机 IP」从侧边栏移到设置页

**现状**：侧边栏顶部显示「本机 IP：192.168.1.10」，有多个 IP 时列出「本机 IP（点击复制）」+ 每行「IP + 网卡名 + 复制图标」。设置页（显示在右侧会话区，最大宽度 640）目前有「数据目录」「开机自动启动」「关于」三个分组，每组是「标题 14/600 + 卡片（popover 底、1 px 边框、内边距 16）」，页面最下方是「退出 Local Echo」。

**要求**：
1. **侧边栏**：删除顶部的本机 IP 区块，侧边栏从「联系人」分组开始。请检查删除后侧边栏顶部的留白是否合适。
2. **设置页**新增分组 **「本机 IP」**，放在第一组（「数据目录」之前）。
3. 分组内用 **Table** 展示，列为「网卡」「IP 地址」「操作」：
   - 「IP 地址」用等宽字体；「操作」列是复制图标按钮（Lucide `copy`），点击后该行图标在 2 秒内变成对勾（`check`，success 色），同时弹出 Toast「已复制 192.168.1.10」。
   - 表头、行高、行线沿用第一版「远程目录浏览器」的表格规格。
   - 网卡名可能很长，例如 `vEthernet (WSL (Hyper-V firewall))`、`enp0s31f6`、`docker0`，需要中间省略或截断。
4. **支持收起 / 展开**：
   - 分组标题行右侧放 chevron（Lucide `chevron-down` / `chevron-right`），整行都可以点击。
   - 收起时，卡片内只显示一行摘要，例如「192.168.1.10 等 3 个」，最常用的 IP 不展开也能看到；只有 1 个 IP 时直接显示这个 IP。
   - 默认展开。
   - 请画出展开、收起两种状态，并标注可点击区域和悬停态。
5. 卡片底部的说明（muted 色）：「联系人添加你时，填写其中任意一个 IP 即可（端口固定为 47210）」。
6. 需要画出的状态：1 个 IP；3–4 个 IP（含长网卡名）；未检测到网络（「未检测到可用的网络连接」）；刚复制某个 IP。

---

# 修改 3：左右两侧底部分割线对齐

**现状**：三处底栏的上边框不在同一水平线上：
- 侧边栏底部的「设置」区：高 52（上下内边距 8 + 36 行高）；
- 联系人会话的输入区（单行时）：高 60（上下内边距 12 + 36）；
- 服务器会话的底栏：高 64（上下内边距 12 + 40 高的拖拽区和按钮）。

**要求**：
- 定义统一的 **底栏高度 token = 60**（不含 1 px 上边框）。
- 侧边栏底部「设置」区：高 60，「设置」行仍为 36，垂直居中。
- 联系人输入区：单行时高 60；输入多行时向上增高（最多约 6 行），这时不要求对齐。
- 服务器底栏：高 60，虚线拖拽区和「上传文件」按钮都改为高 36。
- 在 1000×680 的联系人会话和服务器会话画布上各标出一条对齐参考线，证明左右两侧的分割线在同一像素行。

---

# 修改 4：服务器会话新增「在远程桌面打开」

**功能**：用本机的远程桌面客户端（Windows：系统自带的「远程桌面连接」mstsc；Linux：Remmina 等）以 RDP 协议连接这台服务器。
- 用户名默认使用该服务器的 SSH 用户名；**密码由远程桌面客户端自己询问，Local Echo 不保存**。
- 点击后先检测服务器的远程桌面端口（默认 3389）能否连通，最多 3 秒，再拉起客户端。

**要求**：

a) **会话头部的操作按钮**：现在头部右侧有两个次要按钮「下载文件」（`download`）「在终端中打开」（`square-terminal`）。新增 **「在远程桌面打开」**，图标从 Lucide `monitor` 和 `screen-share` 中选一个，并说明理由。
   - 会话区宽度：窗口最小时为 500（760 − 260），默认 740。三个带文字的按钮约需 400 px，会话区为 500 时放不下，还会挤压左侧的服务器名称和远程目录。
   - 请给出窄宽度方案（例如会话区小于某个宽度时，「在终端中打开」「在远程桌面打开」收成 36×36 的图标按钮并加 Tooltip；你也可以提出更好的方式，但不要把常用操作藏进二级菜单），写明切换阈值，并画出会话区 740 和 500 两种宽度。
   - 按钮状态：常规、悬停、**检测中**（按钮内显示 Spinner，文字为「正在连接…」，期间不能再次点击）。
   - 确认头部同时出现「密钥失效横幅」时布局正常（横幅在头部下方，与这个按钮互不影响）。

b) **失败反馈**：
   - 错误 Toast：「无法连接 192.168.1.20:3389：服务器未开启远程桌面，或端口不可达。可在「编辑服务器」中修改远程桌面端口」（文字较长，请确认 Toast 的换行和最大宽度）。
   - 错误 Toast：「无法启动远程桌面客户端：{原因}」。
   - 对话框「未找到远程桌面客户端」（只在 Linux 上出现）：
     - 说明：「需要安装一个支持 RDP 的远程桌面客户端，推荐 Remmina。」
     - 三行安装命令，等宽字体，每行右侧有复制图标按钮：
       `Ubuntu / Debian：sudo apt install remmina remmina-plugin-rdp`
       `Fedora：sudo dnf install remmina remmina-plugins-rdp`
       `Flatpak：flatpak install flathub org.remmina.Remmina`
     - 按钮：「知道了」。

c) **编辑服务器对话框**：在「用户名」之后新增字段 **「远程桌面端口」**：选填，占位 `3389`，只能输入 1–65535；字段下方提示「用于“在远程桌面打开”，默认 3389」；错误态文案「请输入 1–65535 之间的端口」。「添加服务器」和「重新授权」对话框**不变**。

---

# 实现约束（与第一版相同）

1. 只能使用 gpui-kit 已有的组件形态：TitleBar、Sidebar、Avatar、Badge、Button（primary / secondary / ghost / link / danger，支持 loading）、Input、Textarea、Switch、Dialog、Popover、Tooltip、Notification（Toast）、Alert、Progress、Spinner、**Table**、List、**Collapsible / Accordion**、Setting、Message / Bubble、Attachment、MessageScroller。
2. 图标只用 Lucide。
3. 4 px 栅格；只用纯色和 1 px 细边框；阴影只用于浮层；不用毛玻璃和渐变。
4. 浅色、深色两套都要画；适配 1× 和 2× 缩放；中文文本按比英文长 30% 留余量。
5. 动效克制，只用于悬停、按下、浮层出现、展开 / 收起，时长不超过 150 ms。

---

# 界面文案（请直接使用，可以提出优化建议）

| 位置 | 文案 |
|---|---|
| 文件卡片 | 打开 / 打开文件夹 |
| 文件已不存在 | 文件已被移动或删除 |
| 设置分组 | 本机 IP |
| IP 表格表头 | 网卡 / IP 地址 / 操作 |
| IP 收起摘要 | {ip} 等 {n} 个（只有 1 个时只显示 {ip}） |
| IP 说明 | 联系人添加你时，填写其中任意一个 IP 即可（端口固定为 47210） |
| IP 空状态 | 未检测到可用的网络连接 |
| 复制成功 | 已复制 {ip} |
| 头部按钮 | 在远程桌面打开 |
| 按钮 Tooltip | 用远程桌面连接 {host}:{port} |
| 检测中 | 正在连接… |
| 端口不通 | 无法连接 {host}:{port}：服务器未开启远程桌面，或端口不可达。可在「编辑服务器」中修改远程桌面端口 |
| 拉起失败 | 无法启动远程桌面客户端：{原因} |
| 未找到客户端 | 未找到远程桌面客户端 / 需要安装一个支持 RDP 的远程桌面客户端，推荐 Remmina。/ 知道了 |
| 编辑服务器 | 远程桌面端口 / 用于“在远程桌面打开”，默认 3389 / 请输入 1–65535 之间的端口 |

---

# 交付物

1. **更新后的设计稿**（浅色、深色各一套），只包含有改动的画面：
   - 侧边栏（删除本机 IP 之后，含空分组引导状态）；
   - 联系人会话：文件卡片「已完成」状态（发出 / 收到）、「文件已不存在」状态、底部对齐参考线；
   - 服务器会话：会话区 740 与 500 两种宽度的头部、按钮检测中状态、与密钥失效横幅同时出现、上传 / 下载卡片「已完成」状态、「文件已不存在」状态、底部对齐参考线；
   - 编辑服务器对话框（含远程桌面端口的正常态和错误态）；
   - 设置页：本机 IP 展开 / 收起 / 多 IP / 无网络 / 刚复制；
   - 「未找到远程桌面客户端」对话框；两条错误 Toast。
2. **规格标注更新**：底栏高度 token、文件卡片操作行、IP 表格与收起 / 展开行、头部按钮的窄宽度规则与阈值、新对话框。
3. **变更说明**（半页）：逐条列出改了什么、为什么；如果需要新增颜色 token，请单独列出（尽量不新增）。
4. **不需要**：品牌、Logo、应用图标、托盘图标、主题 JSON 的改动。
````
