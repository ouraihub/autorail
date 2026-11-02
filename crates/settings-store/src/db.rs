//! Opening the database, per-connection pragmas, migration, transactions and row decoding
//! (agent-settings 04 §3.1–§3.2).
//!
//! Turso is async; every call here is driven to completion with `pollster::block_on` on the
//! caller's thread, so no runtime is needed (02 §2, M0).

use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Serialize, de::DeserializeOwned};
use thinkrail_core::{Error, ErrorCode, Result};
use tracing::{info, warn};
use turso::{IntoParams, Row, Value};

use crate::{
    schema::{SCHEMA_VERSION, TABLES},
    store::SqliteSettingsStore,
};

/// How long a connection waits for another connection's write lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// While opening, an engine failure means the file is unusable (`Corrupt`); afterwards it is an
/// operational failure (`External`).
#[derive(Debug, Clone, Copy)]
enum Phase {
    Open,
    Runtime,
}

/// Map an engine error without echoing bound parameters (which may hold keys).
fn engine_error(phase: Phase, error: turso::Error) -> Error {
    if let turso::Error::Constraint(message) = &error
        && message.contains("UNIQUE")
    {
        return Error::conflict("与已有数据重复，请修改后重试。");
    }
    let detail = match &error {
        turso::Error::ToSqlConversionFailure(_) | turso::Error::ConversionFailure(_) => {
            "数据类型转换失败".to_owned()
        }
        other => other.to_string(),
    };
    match phase {
        Phase::Open => Error::corrupt(format!("设置数据库无法打开（{detail}）。")),
        Phase::Runtime => Error::external(format!("设置数据库操作失败（{detail}）。")),
    }
}

fn bad_row() -> Error {
    Error::corrupt("设置数据库中有无法解析的数据。")
}

/// One connection with `foreign_keys` enabled; blocking wrappers over the async API.
pub(crate) struct Conn {
    inner: turso::Connection,
    phase: Phase,
}

impl Conn {
    fn open(db: &turso::Database, phase: Phase) -> Result<Self> {
        let inner = db.connect().map_err(|error| engine_error(phase, error))?;
        inner
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| engine_error(phase, error))?;
        let conn = Self { inner, phase };
        // M0: like SQLite, foreign key enforcement is a per-connection setting.
        conn.execute("PRAGMA foreign_keys = ON", ())?;
        Ok(conn)
    }

    /// Run one statement; returns the number of changed rows.
    pub(crate) fn execute(&self, sql: &str, params: impl IntoParams) -> Result<u64> {
        pollster::block_on(self.inner.execute(sql, params))
            .map_err(|error| engine_error(self.phase, error))
    }

    pub(crate) fn rows(&self, sql: &str, params: impl IntoParams) -> Result<Vec<Row>> {
        pollster::block_on(async {
            let mut rows = self.inner.query(sql, params).await?;
            let mut collected = Vec::new();
            while let Some(row) = rows.next().await? {
                collected.push(row);
            }
            Ok::<_, turso::Error>(collected)
        })
        .map_err(|error| engine_error(self.phase, error))
    }

    pub(crate) fn row(&self, sql: &str, params: impl IntoParams) -> Result<Option<Row>> {
        Ok(self.rows(sql, params)?.into_iter().next())
    }

    /// A query that always yields one integer, such as `COALESCE(MAX(sort), 0) + 1`.
    pub(crate) fn int(&self, sql: &str, params: impl IntoParams) -> Result<i64> {
        int(&self.row(sql, params)?.ok_or_else(bad_row)?, 0)
    }

    /// `body` inside `begin` … `COMMIT`; any failure rolls back.
    fn transaction<T>(&self, begin: &str, body: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.execute(begin, ())?;
        let result = body(self).and_then(|value| self.execute("COMMIT", ()).map(|_| value));
        if result.is_err()
            && !self.inner.is_autocommit().unwrap_or(true)
            && let Err(error) = self.execute("ROLLBACK", ())
        {
            warn!(code = error.code(), "settings rollback failed");
        }
        result
    }
}

impl SqliteSettingsStore {
    /// A consistent snapshot for multi-statement reads.
    pub(crate) fn read<T>(&self, body: impl FnOnce(&Conn) -> Result<T>) -> Result<T> {
        Conn::open(&self.db, Phase::Runtime)?.transaction("BEGIN", body)
    }

    /// A write transaction, serialized with every other writer in this process.
    pub(crate) fn write<T>(&self, body: impl FnOnce(&Conn) -> Result<T>) -> Result<T> {
        let _writer = self.writer.lock();
        Conn::open(&self.db, Phase::Runtime)?.transaction("BEGIN IMMEDIATE", body)
    }
}

/// Create the directory, open the file and migrate it (04 §3.1–§3.2).
pub(crate) fn open(path: &Path) -> Result<turso::Database> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let unwritable = || {
        Error::invalid(format!(
            "无法在 {} 创建数据库，请用 --data-dir 指定可写目录。",
            directory.display()
        ))
    };
    fs::create_dir_all(directory).map_err(|_| unwritable())?;
    let text = path
        .to_str()
        .ok_or_else(|| Error::invalid("数据库路径必须是有效的 Unicode 文本。"))?;
    let db =
        pollster::block_on(turso::Builder::new_local(text).build()).map_err(
            |error| match error {
                turso::Error::IoError(..) | turso::Error::Readonly(_) => unwritable(),
                other => engine_error(Phase::Open, other),
            },
        )?;
    migrate(&Conn::open(&db, Phase::Open)?)?;
    info!(version = SCHEMA_VERSION, "settings database opened");
    Ok(db)
}

/// Empty database: create every table. Same version: use as is. Anything else: `Corrupt`,
/// without writing.
fn migrate(conn: &Conn) -> Result<()> {
    let tables = conn.rows(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        (),
    )?;
    let names = tables
        .iter()
        .map(|row| text(row, 0))
        .collect::<Result<Vec<_>>>()?;
    if names.is_empty() {
        return conn.transaction("BEGIN IMMEDIATE", |conn| {
            for sql in TABLES {
                conn.execute(sql, ())?;
            }
            conn.execute(
                "INSERT INTO schema_meta (version) VALUES (?1)",
                (SCHEMA_VERSION,),
            )?;
            info!(version = SCHEMA_VERSION, "settings database created");
            Ok(())
        });
    }
    if !names.iter().any(|name| name == "schema_meta") {
        return Err(Error::corrupt("这不是 ThinkRail 的设置数据库。"));
    }
    let versions = conn.rows("SELECT version FROM schema_meta", ())?;
    let [row] = versions.as_slice() else {
        return Err(Error::corrupt("设置数据库的版本记录已损坏。"));
    };
    match int(row, 0)? {
        SCHEMA_VERSION => Ok(()),
        version if version > SCHEMA_VERSION => Err(Error::corrupt(
            "数据库由更新版本的 ThinkRail 创建，请升级后再打开。",
        )),
        version if version >= 1 => Err(Error::corrupt(
            "设置数据库来自早期开发版本，表结构已变更；请删除 thinkrail.db 后重新打开。",
        )),
        _ => Err(Error::corrupt("设置数据库的版本号无效。")),
    }
}

pub(crate) fn text(row: &Row, index: usize) -> Result<String> {
    match row.get_value(index) {
        Ok(Value::Text(text)) => Ok(text),
        _ => Err(bad_row()),
    }
}

pub(crate) fn optional_text(row: &Row, index: usize) -> Result<Option<String>> {
    match row.get_value(index) {
        Ok(Value::Text(text)) => Ok(Some(text)),
        Ok(Value::Null) => Ok(None),
        _ => Err(bad_row()),
    }
}

pub(crate) fn int(row: &Row, index: usize) -> Result<i64> {
    match row.get_value(index) {
        Ok(Value::Integer(value)) => Ok(value),
        _ => Err(bad_row()),
    }
}

pub(crate) fn optional_u32(row: &Row, index: usize) -> Result<Option<u32>> {
    match row.get_value(index) {
        Ok(Value::Integer(value)) => u32::try_from(value).map(Some).map_err(|_| bad_row()),
        Ok(Value::Null) => Ok(None),
        _ => Err(bad_row()),
    }
}

pub(crate) fn flag(row: &Row, index: usize) -> Result<bool> {
    Ok(int(row, index)? != 0)
}

pub(crate) fn revision(row: &Row, index: usize) -> Result<u64> {
    u64::try_from(int(row, index)?).map_err(|_| bad_row())
}

/// A JSON column.
pub(crate) fn json<T: DeserializeOwned>(row: &Row, index: usize) -> Result<T> {
    serde_json::from_str(&text(row, index)?).map_err(|_| bad_row())
}

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::invalid("设置无法序列化。"))
}

/// A unit enum stored as its bare serde name, such as `openai-chat`.
pub(crate) fn word<T: DeserializeOwned>(row: &Row, index: usize) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(text(row, index)?)).map_err(|_| bad_row())
}

pub(crate) fn to_word<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(word)) => Ok(word),
        _ => Err(Error::invalid("设置无法序列化。")),
    }
}

/// Milliseconds since the Unix epoch.
pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}
