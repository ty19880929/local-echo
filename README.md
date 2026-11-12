# Local Echo

局域网点对点工具：用聊天窗口与**联系人**互发文本、图片、文件，与**服务器**（SSH）上传、下载文件，并能一键在终端或远程桌面中打开服务器。无服务端、不经公网。

详细设计见 [docs/design.md](docs/design.md)，v0.2 的改造方案见 [docs/plan-v0.2.md](docs/plan-v0.2.md)。

## 开发

```sh
# Linux 构建依赖（Ubuntu / Debian）
sudo apt-get install -y gcc g++ pkg-config libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libvulkan1

cargo run                  # 启动（同一台机器只能运行一个实例）
cargo run -- --hidden      # 只驻留托盘，不打开主窗口（开机自启使用）
```

数据默认存放在 `~/.local/share/local-echo`（Windows：`%LOCALAPPDATA%\local-echo`），可在设置页更换。开发调试时可以用 `XDG_DATA_HOME`、`XDG_CONFIG_HOME` 把数据和配置隔离到临时目录。

### 在 Linux 上交叉编译 Windows 测试版

正式安装包由 CI 在 Windows 上构建；平时测试可以在 Linux 上交叉编译（GNU 工具链），把 exe 拷到 Windows 上直接运行：

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y mingw-w64

scripts/build-windows.sh           # → target/x86_64-pc-windows-gnu/release/local-echo.exe
scripts/build-windows.sh --tests   # 只编译单元测试程序，拷到 Windows 上运行
```

gpui 在 Windows 正式构建时要用 Windows SDK 的 `fxc.exe` 编译着色器，交叉编译时没有这一步，脚本会放入 `packaging/windows/cross/shaders_bytes.rs`（预编译结果，gpui 版本不变就不用重新生成，生成方法见同目录的 `compile-shaders.ps1`）。
## 测试

```sh
cargo test                 # 单元测试 + 联系人协议往返测试（需要 47210 端口空闲）

# 服务器（SSH/SFTP）集成测试：需要一台可用密码登录的 SSH 服务器
LE_SSH=192.168.1.20:22 LE_SSH_USER=user LE_SSH_PASS='***' cargo test ssh_tests -- --ignored
```

## 发布

修改 `Cargo.toml` 中的版本号 → 提交 → `git tag v0.2.0 && git push --tags`。GitHub Actions（`.github/workflows/release.yml`）会构建：

- Windows：Inno Setup 安装程序（`packaging/windows/local-echo.iss`）
- Linux：deb（`cargo deb`）和 rpm（`cargo generate-rpm`），在 ubuntu-22.04 上构建，要求 glibc ≥ 2.35

## 品牌与界面资源

界面按 Claude Design 交付的高保真稿实现，设计源文件归档在 [docs/ui](docs/ui/)（入口：`Local Echo 界面.dc.html`，规格见 `Local Echo 规范.dc.html`，v0.2 的改动见 `Local Echo v0.2.dc.html`）。

| 文件 | 用途 |
|---|---|
| `assets/theme.json` | gpui-kit 主题（Local Echo Light / Dark），跟随系统明暗 |
| `assets/brand/mark*.svg` | 品牌图形（标题栏、空状态、关于），按前景色 / 品牌色着色 |
| `assets/app.ico` | Windows exe 与安装程序图标 |
| `assets/icons/hicolor/…` | Linux 应用图标（48–512 PNG + scalable SVG） |
| `assets/tray/tray-on-{light,dark}[-unread]-{16,20,24,32}.png` | 托盘图标：Windows 按任务栏明暗与缩放选取，Linux 用 on-dark 32 px |

## 在远程桌面打开

服务器会话的「在远程桌面打开」用本机的远程桌面客户端以 RDP 协议连接服务器，用户名预填该服务器的 SSH 用户名，密码由客户端询问，Local Echo 不保存。

- **服务器端**：需要自己开启 RDP 服务，例如 Windows 远程桌面（专业版 / 服务器版）、Linux 的 xrdp、GNOME 远程桌面、KDE KRdp。默认端口 3389，可在「编辑服务器」中修改。
- **Windows 客户端**：使用系统自带的「远程桌面连接」（mstsc），不需要安装。Windows 2026 年 4 月的安全更新之后，打开连接时会先弹「警告：未知的远程连接」，点「连接」继续（第一次使用还需要勾选一次「我了解…」）；需要在远程会话里用剪贴板时，在这个警告里勾选「剪贴板」。
- **Linux 客户端**：依次查找 Remmina（含 Flatpak 版）、KRDC、FreeRDP SDL 客户端（`sdl-freerdp3`）、GNOME Connections 和系统默认的 `rdp://` 处理程序。都没有时会提示安装，推荐 Remmina：

  ```sh
  sudo apt install remmina remmina-plugin-rdp        # Ubuntu / Debian
  sudo dnf install remmina remmina-plugins-rdp       # Fedora
  flatpak install flathub org.remmina.Remmina        # Flatpak
  ```

  deb / rpm 只把它们列为 Suggests，不会自动安装。

## 防火墙

- Windows：安装程序会自动添加 TCP 47210 入站规则。
- Linux：如果开启了防火墙（ufw / firewalld），需要放行 `47210/tcp`。

## 已知限制

- GNOME 远程桌面使用单独设置的 RDP 用户名和密码，与系统账号不同。预填的 SSH 用户名不对时，在客户端的登录框里改成 GNOME 远程桌面的用户名即可。
- xrdp 的已知限制：同一个用户已经在服务器本地登录了图形会话时，用 xrdp 连接会黑屏。先在服务器上注销本地会话，或换一个用户连接。
- Linux X11 会话下，粘贴「已复制的文件」不会触发发送（gpui 的 X11 剪贴板没有实现 `text/uri-list`）。拖拽和附件按钮不受影响；Wayland、Windows 正常。
- 原生 GNOME 需要 AppIndicator 扩展才能显示托盘图标（Ubuntu 默认已启用）。没有托盘时，程序关闭窗口后仍在后台运行：再次启动程序即可唤起窗口，退出在设置页操作。
