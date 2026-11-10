//! 全局共享状态与事件（设计 2.1）。网络任务和托盘只通过 `Event` 通知 UI。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    data,
    db::{Contact, Db, Message, Status},
    host::HostConn,
};

#[derive(Debug)]
pub enum Event {
    /// 收到新消息，或新建了一条传输。
    MessageAdded(Message),
    /// 传输进度（字节），发送端最多每 100ms 发一次。
    Progress {
        id: String,
        done: u64,
    },
    /// 进行中 → 成功 / 失败；失败时附带给用户看的原因。
    Status {
        id: String,
        status: Status,
        error: Option<String>,
    },
    /// 联系人列表有变化（新联系人、IP 自动更新）。
    ContactsChanged,
    /// 服务器密钥登录失败，需要重新授权。
    HostAuthFailed(i64),
    ShowWindow,
    Quit,
}

pub struct Core {
    pub root: PathBuf,
    pub device_id: String,
    pub user: String,
    pub runtime: tokio::runtime::Handle,
    tx: UnboundedSender<Event>,
    /// 迁移数据目录前关闭数据库时为 `None`。
    db: Mutex<Option<Db>>,
    /// 每台服务器缓存的 SSH/SFTP 连接（设计 5.5）。
    pub hosts: tokio::sync::Mutex<HashMap<i64, Arc<HostConn>>>,
}

impl Core {
    pub fn new(
        root: PathBuf,
        db: Db,
        runtime: tokio::runtime::Handle,
        tx: UnboundedSender<Event>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            device_id: data::device_id(&root)?,
            user: data::system_user(),
            root,
            runtime,
            tx,
            db: Mutex::new(Some(db)),
            hosts: Default::default(),
        }))
    }

    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn sender(&self) -> UnboundedSender<Event> {
        self.tx.clone()
    }

    pub fn db<T>(&self, f: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        let guard = self.db.lock().unwrap();
        let db = guard.as_ref().ok_or_else(|| anyhow!("数据库已关闭"))?;
        f(db)
    }

    /// 迁移前关闭数据库连接。
    pub fn close_db(&self) {
        self.db.lock().unwrap().take();
    }

    pub fn files_dir(&self) -> PathBuf {
        self.root.join(data::FILES_DIR)
    }

    pub fn key_path(&self) -> PathBuf {
        self.root.join(data::KEY_FILE)
    }

    pub fn resolve(&self, stored: &str) -> PathBuf {
        data::resolve_stored(&self.root, stored)
    }

    pub fn store(&self, path: &Path) -> String {
        data::to_stored(&self.root, path)
    }

    pub fn contact(&self, id: i64) -> Result<Contact> {
        self.db(|db| db.contact(id))?
            .ok_or_else(|| anyhow!("联系人不存在"))
    }

    /// 写入新消息并通知 UI。
    pub fn add_message(&self, msg: &Message) -> Result<()> {
        self.db(|db| db.upsert_message(msg))?;
        self.emit(Event::MessageAdded(msg.clone()));
        Ok(())
    }

    /// 更新消息状态并通知 UI。
    pub fn finish(&self, id: &str, result: Result<()>) {
        let (status, error) = match result {
            Ok(()) => (Status::Ok, None),
            Err(e) => (Status::Failed, Some(format!("{e:#}"))),
        };
        let _ = self.db(|db| db.set_status(id, status));
        self.emit(Event::Status {
            id: id.to_string(),
            status,
            error,
        });
    }
}
