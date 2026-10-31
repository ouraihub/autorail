//! `ModelPrefService`: default model and effort, hidden models (agent-settings 04 §2.4).

use thinkrail_core::{Error, ModelPrefService, ModelPrefs, ModelRef, Result, ThinkingLevel};
use tracing::info;

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
};

const DEFAULT_MODEL: &str = "default_model";
const DEFAULT_EFFORT: &str = "default_effort";

fn load(conn: &Conn) -> Result<ModelPrefs> {
    let mut prefs = ModelPrefs::default();
    for row in conn.rows("SELECT key, value FROM model_prefs", ())? {
        match db::text(&row, 0)?.as_str() {
            DEFAULT_MODEL => prefs.default_model = Some(db::json(&row, 1)?),
            DEFAULT_EFFORT => prefs.default_effort = Some(db::json(&row, 1)?),
            _ => {}
        }
    }
    prefs.hidden = conn
        .rows("SELECT provider_id, model_id FROM hidden_models ORDER BY provider_id, model_id", ())?
        .iter()
        .map(|row| Ok(ModelRef { provider: db::text(row, 0)?.into(), model_id: db::text(row, 1)? }))
        .collect::<Result<_>>()?;
    Ok(prefs)
}

fn ensure_model(conn: &Conn, model: &ModelRef) -> Result<()> {
    let sql = "SELECT 1 FROM provider_models WHERE provider_id = ?1 AND model_id = ?2";
    if conn.row(sql, (model.provider.as_str(), model.model_id.as_str()))?.is_none() {
        return Err(Error::not_found(format!("模型 {} 不存在，可能已被删除。", model.model_id)));
    }
    Ok(())
}

fn put<T: serde::Serialize>(conn: &Conn, key: &str, value: Option<&T>) -> Result<()> {
    match value {
        Some(value) => conn.execute(
            "INSERT INTO model_prefs (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (key, db::to_json(value)?),
        )?,
        None => conn.execute("DELETE FROM model_prefs WHERE key = ?1", (key,))?,
    };
    Ok(())
}

impl ModelPrefService for SqliteSettingsStore {
    fn prefs(&self) -> Result<ModelPrefs> {
        self.read(load)
    }

    fn set_default(&self, model: Option<ModelRef>, effort: Option<ThinkingLevel>) -> Result<ModelPrefs> {
        let prefs = self.write(|conn| {
            if let Some(model) = &model {
                ensure_model(conn, model)?;
            }
            put(conn, DEFAULT_MODEL, model.as_ref())?;
            put(conn, DEFAULT_EFFORT, effort.as_ref())?;
            load(conn)
        })?;
        info!(provider = prefs.default_model.as_ref().map(|model| model.provider.as_str()), "default model set");
        Ok(prefs)
    }

    fn set_hidden(&self, model: &ModelRef, hidden: bool) -> Result<ModelPrefs> {
        self.write(|conn| {
            let params = (model.provider.as_str(), model.model_id.as_str());
            if hidden {
                ensure_model(conn, model)?;
                conn.execute("INSERT OR IGNORE INTO hidden_models (provider_id, model_id) VALUES (?1, ?2)", params)?;
            } else {
                conn.execute("DELETE FROM hidden_models WHERE provider_id = ?1 AND model_id = ?2", params)?;
            }
            load(conn)
        })
    }
}
