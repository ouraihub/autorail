//! `AppPrefService`: host-side preferences in `app_prefs` (native-multi-agent 06 §4).

use thinkrail_core::{AppPrefService, AppPrefs, LogLevel, Result};
use tracing::info;

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
};

const LOGS_VISIBLE: &str = "logs.visible";
const LOG_LEVEL: &str = "logs.level";

fn load(conn: &Conn) -> Result<AppPrefs> {
    let mut prefs = AppPrefs::default();
    for row in conn.rows("SELECT key, value FROM app_prefs", ())? {
        match db::text(&row, 0)?.as_str() {
            LOGS_VISIBLE => prefs.logs_visible = db::json(&row, 1)?,
            LOG_LEVEL => prefs.log_level = db::json(&row, 1)?,
            _ => {}
        }
    }
    Ok(prefs)
}

fn put<T: serde::Serialize>(conn: &Conn, key: &str, value: &T) -> Result<()> {
    conn.execute(
        "INSERT INTO app_prefs (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        (key, db::to_json(value)?),
    )?;
    Ok(())
}

impl AppPrefService for SqliteSettingsStore {
    fn app_prefs(&self) -> Result<AppPrefs> {
        self.read(load)
    }

    fn set_logs_visible(&self, visible: bool) -> Result<AppPrefs> {
        let prefs = self.write(|conn| {
            put(conn, LOGS_VISIBLE, &visible)?;
            load(conn)
        })?;
        info!(visible, "log page visibility set");
        Ok(prefs)
    }

    fn set_log_level(&self, level: LogLevel) -> Result<AppPrefs> {
        let prefs = self.write(|conn| {
            put(conn, LOG_LEVEL, &level)?;
            load(conn)
        })?;
        info!(level = level.as_str(), "log level stored");
        Ok(prefs)
    }
}
