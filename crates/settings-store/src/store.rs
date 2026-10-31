//! The store handle shared by the three service implementations (agent-settings 04 §3.1).

use std::{collections::HashMap, path::Path};

use parking_lot::Mutex;
use thinkrail_core::{ProviderId, ProviderKeyId, Result};

use crate::db;

/// [`thinkrail_core::ProviderService`], [`thinkrail_core::McpService`] and
/// [`thinkrail_core::ModelPrefService`] backed by one Turso database file.
///
/// Methods block; call them from a background executor. Every call opens its own connection on
/// the shared `db` (safe across threads), and writes are serialized in-process by `writer`.
#[derive(Debug)]
pub struct SqliteSettingsStore {
    pub(crate) db: turso::Database,
    /// Serializes write transactions in this process so they queue instead of reporting busy.
    pub(crate) writer: Mutex<()>,
    /// Last key handed out per provider by `next_key`; memory only, so a restart begins at the first key.
    pub(crate) cursor: Mutex<HashMap<ProviderId, ProviderKeyId>>,
}

impl SqliteSettingsStore {
    /// Open the database at `path` (normally `<data>/thinkrail.db`), creating the file and its
    /// directory when missing and migrating an empty database. A database written by a newer
    /// version fails with `Corrupt` and is left untouched.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self { db: db::open(path)?, writer: Mutex::new(()), cursor: Mutex::new(HashMap::new()) })
    }
}
