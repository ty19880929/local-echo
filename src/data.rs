//! 数据目录（设计 3.1、3.3、7.2）：指针文件、设备 ID、文件命名、迁移。

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

/// 与人交互的固定端口（设计 4.1）。
pub const PORT: u16 = 47210;
pub const DB_FILE: &str = "local-echo.db";
pub const DEVICE_ID_FILE: &str = "device_id";
pub const KEY_FILE: &str = "id_ed25519";
pub const FILES_DIR: &str = "files";

const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

fn pointer_path() -> PathBuf {
    dirs::config_dir()
        .expect("no config dir")
        .join("local-echo")
        .join("data_dir")
}

fn default_root() -> PathBuf {
    dirs::data_local_dir()
        .expect("no local data dir")
        .join("local-echo")
}

/// 读取指针文件得到数据目录；没有指针文件时使用默认位置。
pub fn resolve_root() -> PathBuf {
    fs::read_to_string(pointer_path())
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(default_root)
}

/// 改写指针文件。
pub fn set_root(root: &Path) -> Result<()> {
    let pointer = pointer_path();
    fs::create_dir_all(pointer.parent().unwrap())?;
    fs::write(pointer, root.to_string_lossy().as_bytes())?;
    Ok(())
}

pub fn ensure_layout(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join(FILES_DIR))
        .with_context(|| format!("无法创建数据目录 {}", root.display()))
}

/// 读取本机设备 ID，不存在时生成。
pub fn device_id(root: &Path) -> Result<String> {
    let path = root.join(DEVICE_ID_FILE);
    if let Ok(id) = fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    fs::write(&path, &id)?;
    Ok(id)
}

/// 系统用户名：被对方自动建联系人时的默认昵称。
pub fn system_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

pub fn is_image(name: &str) -> bool {
    extension(name).is_some_and(|ext| IMAGE_EXTS.contains(&ext.as_str()))
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// 图片按保存时刻的本地时间命名：`YYYYMMDD-HHMMSS-mmm.<扩展名>`（设计 3.3）。
pub fn image_name(original: &str) -> String {
    let ext = extension(original).unwrap_or_else(|| "png".into());
    format!("{}.{ext}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"))
}

/// 只取最后一段；Windows 上替换非法字符。
pub fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| {
            let illegal = cfg!(windows) && matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|');
            if illegal || c.is_control() { '_' } else { c }
        })
        .collect();
    match cleaned.trim() {
        "" | "." | ".." => "file".into(),
        _ => cleaned,
    }
}

/// 在目录中以 `name` 创建新文件，重名时追加 ` (1)`、` (2)`……
/// 用 `create_new` 原子占位，并发接收同名文件也不会互相覆盖。
pub fn create_unique(dir: &Path, name: &str) -> Result<(PathBuf, File)> {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned());
    for i in 0u32.. {
        let candidate = match (i, &ext) {
            (0, _) => name.to_string(),
            (_, Some(ext)) => format!("{stem} ({i}).{ext}"),
            (_, None) => format!("{stem} ({i})"),
        };
        let path = dir.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("无法创建 {}", path.display())),
        }
    }
    unreachable!()
}

/// 数据目录内的文件存相对路径（`files/xxx`），目录外的存绝对路径。
pub fn to_stored(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// 相对路径（统一按 `/` 存储）按段拼接，得到全部使用本机分隔符的路径。
/// Windows 的 Shell 不接受混用 `/` 的路径，直接 `join` 会让「打开文件夹」失败（v0.2 B1）。
pub fn resolve_stored(root: &Path, stored: &str) -> PathBuf {
    let path = Path::new(stored);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let mut resolved = root.to_path_buf();
    for part in stored.split('/').filter(|p| !p.is_empty()) {
        resolved.push(part);
    }
    resolved
}

/// 收紧私钥权限，否则 OpenSSH 拒绝使用（设计 5.6）。
pub fn restrict_key(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let grant = format!("{}:F", system_user());
        std::process::Command::new("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &grant])
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
    }
    Ok(())
}

pub fn has_data(dir: &Path) -> bool {
    dir.join(DB_FILE).is_file()
}

/// 把数据从 `from` 移动到 `to`（设计 7.2）。同一分区直接 rename，否则复制后删除。
/// 调用前必须已关闭数据库。`progress(done, total)` 报告复制进度（字节）。
pub fn migrate(from: &Path, to: &Path, progress: impl Fn(u64, u64)) -> Result<()> {
    fs::create_dir_all(to)?;
    let items: Vec<&str> = [
        DB_FILE,
        "local-echo.db-wal",
        "local-echo.db-shm",
        DEVICE_ID_FILE,
        KEY_FILE,
        FILES_DIR,
    ]
    .into_iter()
    .filter(|item| from.join(item).exists())
    .collect();

    let total: u64 = items.iter().map(|item| size_of(&from.join(item))).sum();
    let mut done = 0u64;
    for item in items {
        let (src, dst) = (from.join(item), to.join(item));
        if fs::rename(&src, &dst).is_ok() {
            done += size_of(&dst);
            progress(done, total);
            continue;
        }
        copy_recursive(&src, &dst, &mut |n| {
            done += n;
            progress(done, total);
        })?;
        if src.is_dir() {
            fs::remove_dir_all(&src)?
        } else {
            fs::remove_file(&src)?
        }
    }
    let key = to.join(KEY_FILE);
    if key.exists() {
        restrict_key(&key)?;
    }
    Ok(())
}

fn size_of(path: &Path) -> u64 {
    if path.is_dir() {
        fs::read_dir(path)
            .map(|entries| entries.flatten().map(|e| size_of(&e.path())).sum())
            .unwrap_or(0)
    } else {
        path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

fn copy_recursive(src: &Path, dst: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()), on_bytes)?;
        }
    } else {
        on_bytes(fs::copy(src, dst)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\Users\\a\\报告.pdf"), "报告.pdf");
        assert_eq!(sanitize_file_name(".."), "file");
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("le-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (a, _) = create_unique(&dir, "a.txt").unwrap();
        let (b, _) = create_unique(&dir, "a.txt").unwrap();
        let (c, _) = create_unique(&dir, "noext").unwrap();
        let (d, _) = create_unique(&dir, "noext").unwrap();
        assert_eq!(a.file_name().unwrap(), "a.txt");
        assert_eq!(b.file_name().unwrap(), "a (1).txt");
        assert_eq!(c.file_name().unwrap(), "noext");
        assert_eq!(d.file_name().unwrap(), "noext (1)");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn image_names() {
        assert!(is_image("A.JPG"));
        assert!(!is_image("a.txt"));
        let name = image_name("x.JPEG");
        assert!(name.ends_with(".jpeg") && name.len() == "20261109-143205-123.jpeg".len());
    }

    #[test]
    fn stored_paths() {
        let root = Path::new("/data/le");
        assert_eq!(
            to_stored(root, Path::new("/data/le/files/a.png")),
            "files/a.png"
        );
        assert_eq!(to_stored(root, Path::new("/home/a.png")), "/home/a.png");
        assert_eq!(
            resolve_stored(root, "files/a.png"),
            Path::new("/data/le/files/a.png")
        );
    }

    /// v0.2 B1：相对路径按段拼接，Windows 上结果不能混入 `/`。
    #[test]
    fn stored_paths_use_native_separators() {
        let root = std::env::temp_dir().join("le data").join("echo");
        let resolved = resolve_stored(&root, "files/报告 v2.pdf");
        assert_eq!(resolved, root.join("files").join("报告 v2.pdf"));
        assert_eq!(
            resolve_stored(&root, "files/sub/a.txt"),
            root.join("files").join("sub").join("a.txt")
        );
        #[cfg(windows)]
        assert!(
            !resolved.to_string_lossy().contains('/'),
            "{}",
            resolved.display()
        );
        // 往返：to_stored 存相对路径，resolve_stored 还原成同一个文件。
        let stored = to_stored(&root, &resolved);
        assert_eq!(stored, "files/报告 v2.pdf");
        assert_eq!(resolve_stored(&root, &stored), resolved);
    }
}
