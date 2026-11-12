//! SQLite 存储（设计 3.2）。

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContactKind {
    Peer = 0,
    Host = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgKind {
    Text = 0,
    Image = 1,
    File = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending = 0,
    Ok = 1,
    Failed = 2,
}

#[derive(Clone, Debug)]
pub struct Contact {
    pub id: i64,
    pub kind: ContactKind,
    /// 联系人昵称 / 服务器名称。
    pub name: String,
    /// 联系人当前 IP / 服务器地址。
    pub host: String,
    pub port: u16,
    /// 服务器 SSH 用户名。
    pub user: String,
    /// 联系人的设备 ID（唯一标识）。
    pub peer_id: Option<String>,
    /// 服务器当前远程目录。
    pub remote_dir: String,
    /// 服务器的远程桌面（RDP）端口，默认 3389；联系人不使用（v0.2 F1）。
    pub rdp_port: u16,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub contact_id: i64,
    pub outgoing: bool,
    pub kind: MsgKind,
    /// 文本内容或文件名。
    pub body: String,
    pub path: Option<String>,
    pub remote_path: Option<String>,
    pub size: Option<u64>,
    pub status: Status,
    pub ts: i64,
}

/// 同名文件确认的判断条件（设计 6）。
pub enum SameName<'a> {
    /// 联系人：曾成功发送过同名文件。
    FileName(&'a str),
    /// 服务器：曾成功上传到同一远程路径。
    RemotePath(&'a str),
}

pub struct Db(Connection);

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS contact (
  id         INTEGER PRIMARY KEY,
  kind       INTEGER NOT NULL,
  name       TEXT    NOT NULL,
  host       TEXT    NOT NULL,
  port       INTEGER NOT NULL,
  user       TEXT    NOT NULL DEFAULT '',
  peer_id    TEXT,
  remote_dir TEXT    NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX IF NOT EXISTS contact_peer ON contact (peer_id) WHERE kind = 0;
CREATE TABLE IF NOT EXISTS message (
  id          TEXT    PRIMARY KEY,
  contact_id  INTEGER NOT NULL REFERENCES contact(id) ON DELETE CASCADE,
  outgoing    INTEGER NOT NULL,
  kind        INTEGER NOT NULL,
  body        TEXT    NOT NULL,
  path        TEXT,
  remote_path TEXT,
  size        INTEGER,
  status      INTEGER NOT NULL,
  ts          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS message_by_contact ON message (contact_id);
";

/// 数据库结构版本（`PRAGMA user_version`）：0 = v0.1，1 = v0.2（contact.rdp_port）。
const SCHEMA_VERSION: i64 = 1;

/// 远程桌面默认端口。
pub const DEFAULT_RDP_PORT: u16 = 3389;

const CONTACT_COLS: &str = "id, kind, name, host, port, user, peer_id, remote_dir, rdp_port";
const MESSAGE_COLS: &str =
    "id, contact_id, outgoing, kind, body, path, remote_path, size, status, ts";

pub fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        upgrade(&conn)?;
        // 上次异常退出时未完成的传输标为失败（设计 3.2）。
        conn.execute("UPDATE message SET status = 2 WHERE status = 0", [])?;
        Ok(Self(conn))
    }

    pub fn contacts(&self) -> Result<Vec<Contact>> {
        let sql = format!(
            "SELECT {CONTACT_COLS} FROM contact ORDER BY \
             (SELECT max(rowid) FROM message WHERE contact_id = contact.id) DESC, id"
        );
        let mut stmt = self.0.prepare(&sql)?;
        let rows = stmt.query_map([], contact_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn contact(&self, id: i64) -> Result<Option<Contact>> {
        let sql = format!("SELECT {CONTACT_COLS} FROM contact WHERE id = ?1");
        Ok(self.0.query_row(&sql, [id], contact_from_row).optional()?)
    }

    pub fn contact_by_peer(&self, peer_id: &str) -> Result<Option<Contact>> {
        let sql = format!("SELECT {CONTACT_COLS} FROM contact WHERE kind = 0 AND peer_id = ?1");
        Ok(self
            .0
            .query_row(&sql, [peer_id], contact_from_row)
            .optional()?)
    }

    pub fn insert_contact(&self, c: &Contact) -> Result<i64> {
        self.0.execute(
            "INSERT INTO contact (kind, name, host, port, user, peer_id, remote_dir, rdp_port) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                c.kind as i64,
                c.name,
                c.host,
                c.port,
                c.user,
                c.peer_id,
                c.remote_dir,
                c.rdp_port
            ],
        )?;
        Ok(self.0.last_insert_rowid())
    }

    pub fn update_contact(&self, c: &Contact) -> Result<()> {
        self.0.execute(
            "UPDATE contact SET name = ?2, host = ?3, port = ?4, user = ?5, remote_dir = ?6, \
             rdp_port = ?7 WHERE id = ?1",
            params![
                c.id,
                c.name,
                c.host,
                c.port,
                c.user,
                c.remote_dir,
                c.rdp_port
            ],
        )?;
        Ok(())
    }

    pub fn delete_contact(&self, id: i64) -> Result<()> {
        self.0.execute("DELETE FROM contact WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<Message>> {
        let sql =
            format!("SELECT {MESSAGE_COLS} FROM message WHERE contact_id = ?1 ORDER BY rowid");
        let mut stmt = self.0.prepare(&sql)?;
        let rows = stmt.query_map([contact_id], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn message(&self, id: &str) -> Result<Option<Message>> {
        let sql = format!("SELECT {MESSAGE_COLS} FROM message WHERE id = ?1");
        Ok(self.0.query_row(&sql, [id], message_from_row).optional()?)
    }

    /// 新增消息；同一 id 已存在时（对方重试）重置状态和路径（设计 4.4）。
    pub fn upsert_message(&self, m: &Message) -> Result<()> {
        self.0.execute(
            "INSERT INTO message (id, contact_id, outgoing, kind, body, path, remote_path, size, status, ts) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(id) DO UPDATE SET status = excluded.status, path = excluded.path, body = excluded.body",
            params![
                m.id,
                m.contact_id,
                m.outgoing,
                m.kind as i64,
                m.body,
                m.path,
                m.remote_path,
                m.size.map(|s| s as i64),
                m.status as i64,
                m.ts
            ],
        )?;
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: Status) -> Result<()> {
        self.0.execute(
            "UPDATE message SET status = ?2 WHERE id = ?1",
            params![id, status as i64],
        )?;
        Ok(())
    }

    /// 是否有进行中的传输（迁移前检查，设计 7.2）。
    pub fn has_pending(&self) -> Result<bool> {
        let n: i64 =
            self.0
                .query_row("SELECT count(*) FROM message WHERE status = 0", [], |r| {
                    r.get(0)
                })?;
        Ok(n > 0)
    }

    /// 最近一次成功发送 / 上传同名文件的时间（设计 6）。
    pub fn last_sent(&self, contact_id: i64, key: SameName) -> Result<Option<i64>> {
        let (cond, value) = match key {
            SameName::FileName(name) => ("kind = 2 AND body = ?2", name),
            SameName::RemotePath(path) => ("remote_path = ?2", path),
        };
        let sql = format!(
            "SELECT ts FROM message WHERE contact_id = ?1 AND outgoing = 1 AND status = 1 AND {cond} \
             ORDER BY rowid DESC LIMIT 1"
        );
        Ok(self
            .0
            .query_row(&sql, params![contact_id, value], |r| r.get(0))
            .optional()?)
    }
}

/// 按 `user_version` 逐级升级表结构；新库先按 v0.1 结构建表，再走同一条升级路径。
/// 只做加列：v0.1 的 SELECT / INSERT 都写了明确的列名，打开升级后的库不受影响。
fn upgrade(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        let has_rdp_port = conn
            .prepare("SELECT 1 FROM pragma_table_info('contact') WHERE name = 'rdp_port'")?
            .exists([])?;
        if !has_rdp_port {
            conn.execute_batch(&format!(
                "ALTER TABLE contact ADD COLUMN rdp_port INTEGER NOT NULL DEFAULT {DEFAULT_RDP_PORT}"
            ))?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn contact_from_row(row: &Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        kind: if row.get::<_, i64>(1)? == 0 {
            ContactKind::Peer
        } else {
            ContactKind::Host
        },
        name: row.get(2)?,
        host: row.get(3)?,
        port: row.get(4)?,
        user: row.get(5)?,
        peer_id: row.get(6)?,
        remote_dir: row.get(7)?,
        rdp_port: row.get(8)?,
    })
}

fn message_from_row(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        contact_id: row.get(1)?,
        outgoing: row.get(2)?,
        kind: match row.get::<_, i64>(3)? {
            0 => MsgKind::Text,
            1 => MsgKind::Image,
            _ => MsgKind::File,
        },
        body: row.get(4)?,
        path: row.get(5)?,
        remote_path: row.get(6)?,
        size: row.get::<_, Option<i64>>(7)?.map(|s| s as u64),
        status: match row.get::<_, i64>(8)? {
            0 => Status::Pending,
            1 => Status::Ok,
            _ => Status::Failed,
        },
        ts: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> Db {
        Db::open(Path::new(":memory:")).unwrap()
    }

    fn peer(name: &str, peer_id: &str) -> Contact {
        Contact {
            id: 0,
            kind: ContactKind::Peer,
            name: name.into(),
            host: "10.0.0.2".into(),
            port: 47210,
            user: String::new(),
            peer_id: Some(peer_id.into()),
            remote_dir: String::new(),
            rdp_port: DEFAULT_RDP_PORT,
        }
    }

    fn msg(id: &str, contact_id: i64, kind: MsgKind, body: &str, status: Status) -> Message {
        Message {
            id: id.into(),
            contact_id,
            outgoing: true,
            kind,
            body: body.into(),
            path: None,
            remote_path: None,
            size: Some(1),
            status,
            ts: now_ms(),
        }
    }

    #[test]
    fn peer_identity_is_unique() {
        let db = temp_db();
        db.insert_contact(&peer("a", "dev-1")).unwrap();
        assert!(db.insert_contact(&peer("b", "dev-1")).is_err());
        assert_eq!(db.contact_by_peer("dev-1").unwrap().unwrap().name, "a");
    }

    #[test]
    fn same_name_only_counts_successful_files() {
        let db = temp_db();
        let id = db.insert_contact(&peer("a", "dev-1")).unwrap();
        db.upsert_message(&msg("1", id, MsgKind::Image, "x.png", Status::Ok))
            .unwrap();
        db.upsert_message(&msg("2", id, MsgKind::File, "r.pdf", Status::Failed))
            .unwrap();
        assert_eq!(db.last_sent(id, SameName::FileName("x.png")).unwrap(), None);
        assert_eq!(db.last_sent(id, SameName::FileName("r.pdf")).unwrap(), None);
        db.set_status("2", Status::Ok).unwrap();
        assert!(
            db.last_sent(id, SameName::FileName("r.pdf"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn delete_cascades_messages() {
        let db = temp_db();
        let id = db.insert_contact(&peer("a", "dev-1")).unwrap();
        db.upsert_message(&msg("1", id, MsgKind::Text, "hi", Status::Ok))
            .unwrap();
        db.delete_contact(id).unwrap();
        assert!(db.message("1").unwrap().is_none());
    }

    #[test]
    fn upgrades_v01_schema() {
        let path = std::env::temp_dir().join(format!("le-db-{}.db", uuid::Uuid::new_v4()));
        {
            // v0.1 的表结构与写入方式（没有 rdp_port 列，user_version = 0）。
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO contact (kind, name, host, port, user, peer_id, remote_dir) \
                 VALUES (1, 'srv', '10.0.0.5', 22, 'root', NULL, '/root')",
                [],
            )
            .unwrap();
        }
        for _ in 0..2 {
            // 第二次打开时已是 v1，不能重复加列。
            let db = Db::open(&path).unwrap();
            let version: i64 =
                db.0.pragma_query_value(None, "user_version", |r| r.get(0))
                    .unwrap();
            assert_eq!(version, SCHEMA_VERSION);
            let mut host = db.contacts().unwrap().remove(0);
            assert_eq!(host.rdp_port, DEFAULT_RDP_PORT);
            host.rdp_port = 13389;
            db.update_contact(&host).unwrap();
            assert_eq!(db.contact(host.id).unwrap().unwrap().rdp_port, 13389);
            host.rdp_port = DEFAULT_RDP_PORT;
            db.update_contact(&host).unwrap();
        }
        {
            // v0.1 程序按明确列名读写升级后的库（降级、换电脑迁移）。
            let conn = Connection::open(&path).unwrap();
            let name: String = conn
                .query_row(
                    "SELECT id, kind, name, host, port, user, peer_id, remote_dir FROM contact",
                    [],
                    |r| r.get(2),
                )
                .unwrap();
            assert_eq!(name, "srv");
            conn.execute(
                "INSERT INTO contact (kind, name, host, port, user, peer_id, remote_dir) \
                 VALUES (1, 'b', '10.0.0.6', 22, 'u', NULL, '/')",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert!(
            db.contacts()
                .unwrap()
                .iter()
                .all(|c| c.rdp_port == DEFAULT_RDP_PORT)
        );
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn new_database_is_current() {
        let db = temp_db();
        let version: i64 =
            db.0.pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let id = db.insert_contact(&peer("a", "dev-1")).unwrap();
        assert_eq!(db.contact(id).unwrap().unwrap().rdp_port, DEFAULT_RDP_PORT);
    }
}
