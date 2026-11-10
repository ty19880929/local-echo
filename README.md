# Local Echo

局域网点对点工具：用聊天窗口与**联系人**互发文本、图片、文件，与**服务器**（SSH）上传、下载文件，并能一键在终端中打开服务器。无服务端、不经公网。

详细设计见 [docs/design.md](docs/design.md)（v1.0 定稿）。

## 开发

```sh
# Linux 构建依赖（Ubuntu / Debian）
sudo apt-get install -y gcc g++ pkg-config libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libvulkan1

cargo run                  # 启动（同一台机器只能运行一个实例）
cargo run -- --hidden      # 只驻留托盘，不打开主窗口（开机自启使用）
```

数据默认存放在 `~/.local/share/local-echo`（Windows：`%LOCALAPPDATA%\local-echo`），可在设置页更换。开发调试时可以用 `XDG_DATA_HOME`、`XDG_CONFIG_HOME` 把数据和配置隔离到临时目录。

## 测试

```sh
cargo test                 # 单元测试 + 联系人协议往返测试（需要 47210 端口空闲）

# 服务器（SSH/SFTP）集成测试：需要一台可用密码登录的 SSH 服务器
LE_SSH=192.168.1.20:22 LE_SSH_USER=user LE_SSH_PASS='***' cargo test ssh_tests -- --ignored
```

## 发布

修改 `Cargo.toml` 中的版本号 → 提交 → `git tag v0.1.0 && git push --tags`。GitHub Actions（`.github/workflows/release.yml`）会构建：

- Windows：Inno Setup 安装程序（`packaging/windows/local-echo.iss`）
- Linux：deb（`cargo deb`）和 rpm（`cargo generate-rpm`），在 ubuntu-22.04 上构建，要求 glibc ≥ 2.35

## 品牌与界面资源

`assets/` 下的图标目前是占位图。Claude Design 产出后，按同样的文件名替换即可：

| 文件 | 用途 |
|---|---|
| `assets/app.ico` | Windows exe 和安装程序图标 |
| `assets/icons/hicolor/{48,128,256,512}x…/apps/local-echo.png` | Linux 应用图标 |
| `assets/tray.png` / `assets/tray-unread.png` | 托盘图标（常规 / 有未读），32×32 |

## 防火墙

- Windows：安装程序会自动添加 TCP 47210 入站规则。
- Linux：如果开启了防火墙（ufw / firewalld），需要放行 `47210/tcp`。

## 已知限制

- Linux X11 会话下，粘贴「已复制的文件」不会触发发送（gpui 的 X11 剪贴板没有实现 `text/uri-list`）。拖拽和附件按钮不受影响；Wayland、Windows 正常。
- 原生 GNOME 需要 AppIndicator 扩展才能显示托盘图标（Ubuntu 默认已启用）。没有托盘时，程序关闭窗口后仍在后台运行：再次启动程序即可唤起窗口，退出在设置页操作。
