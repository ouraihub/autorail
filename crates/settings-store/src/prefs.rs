//! `ModelPrefService`: default model and effort, hidden models, 编码搭档 choices, the default
//! Agent, context-window overrides and the 评审 settings (agent-settings 04 §2.4, 07, 08).

use thinkrail_core::{
    BackendKind, CONTEXT_OVERRIDE_RANGE, EngineProvider, Error, ModelPrefService, ModelPrefs,
    ModelRef, Result, ReviewSwitch, ThinkingLevel,
};
use tracing::info;

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
};

const DEFAULT_MODEL: &str = "default_model";
const DEFAULT_EFFORT: &str = "default_effort";
const DEFAULT_AGENT: &str = "default_agent";
/// 评审 group (agent-settings 08 §5.1); both switches are off when absent.
const REVIEW_MODEL: &str = "review_model";
const REVIEW_EFFORT: &str = "review_effort";
const AGENT_REVIEW: &str = "agent_review";
const REVIEW_AUTO_FIX: &str = "review_auto_fix";
/// `engine_provider.pi` etc.; the value is a JSON [`EngineProvider`].
const ENGINE_PROVIDER: &str = "engine_provider.";
/// `context_override.<provider id>.<model id>`; the value is a token count (agent-settings 08 §6).
/// Provider ids are UUIDs without `.`, so the first `.` after the prefix splits the two; the
/// model id keeps any `.`, `/` or `:` it has.
pub(crate) const CONTEXT_OVERRIDE: &str = "context_override.";

fn context_key(model: &ModelRef) -> String {
    format!(
        "{CONTEXT_OVERRIDE}{}.{}",
        model.provider.as_str(),
        model.model_id
    )
}

fn parse_context_key(rest: &str) -> Result<ModelRef> {
    let (provider, model_id) = rest
        .split_once('.')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
        .ok_or_else(|| Error::corrupt("模型偏好里有无法识别的上下文覆盖。"))?;
    Ok(ModelRef {
        provider: provider.into(),
        model_id: model_id.to_owned(),
    })
}

fn load(conn: &Conn) -> Result<ModelPrefs> {
    let mut prefs = ModelPrefs::default();
    for row in conn.rows("SELECT key, value FROM model_prefs", ())? {
        match db::text(&row, 0)?.as_str() {
            DEFAULT_MODEL => prefs.default_model = Some(db::json(&row, 1)?),
            DEFAULT_EFFORT => prefs.default_effort = Some(db::json(&row, 1)?),
            DEFAULT_AGENT => prefs.default_agent = Some(db::json(&row, 1)?),
            REVIEW_MODEL => prefs.review.model = Some(db::json(&row, 1)?),
            REVIEW_EFFORT => prefs.review.effort = Some(db::json(&row, 1)?),
            AGENT_REVIEW => prefs.review.agent_review = db::json(&row, 1)?,
            REVIEW_AUTO_FIX => prefs.review.auto_fix = db::json(&row, 1)?,
            key => {
                if let Some(backend) = key.strip_prefix(ENGINE_PROVIDER) {
                    let backend: BackendKind =
                        serde_json::from_value(serde_json::Value::String(backend.to_owned()))
                            .map_err(|_| Error::corrupt("模型偏好里有无法识别的引擎。"))?;
                    prefs.engine_providers.push((backend, db::json(&row, 1)?));
                } else if let Some(rest) = key.strip_prefix(CONTEXT_OVERRIDE) {
                    prefs
                        .context_overrides
                        .push((parse_context_key(rest)?, db::json(&row, 1)?));
                }
            }
        }
    }
    prefs.hidden = conn
        .rows(
            "SELECT provider_id, model_id FROM hidden_models ORDER BY provider_id, model_id",
            (),
        )?
        .iter()
        .map(|row| {
            Ok(ModelRef {
                provider: db::text(row, 0)?.into(),
                model_id: db::text(row, 1)?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(prefs)
}

fn ensure_model(conn: &Conn, model: &ModelRef) -> Result<()> {
    let sql = "SELECT 1 FROM provider_models WHERE provider_id = ?1 AND model_id = ?2";
    if conn
        .row(sql, (model.provider.as_str(), model.model_id.as_str()))?
        .is_none()
    {
        return Err(Error::not_found(format!(
            "模型 {} 不存在，可能已被删除。",
            model.model_id
        )));
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

    fn set_default(
        &self,
        model: Option<ModelRef>,
        effort: Option<ThinkingLevel>,
    ) -> Result<ModelPrefs> {
        let prefs = self.write(|conn| {
            if let Some(model) = &model {
                ensure_model(conn, model)?;
            }
            put(conn, DEFAULT_MODEL, model.as_ref())?;
            put(conn, DEFAULT_EFFORT, effort.as_ref())?;
            load(conn)
        })?;
        info!(
            provider = prefs
                .default_model
                .as_ref()
                .map(|model| model.provider.as_str()),
            "default model set"
        );
        Ok(prefs)
    }

    fn set_engine_provider(
        &self,
        backend: BackendKind,
        choice: EngineProvider,
    ) -> Result<ModelPrefs> {
        let prefs = self.write(|conn| {
            if let EngineProvider::Provider(id) = &choice {
                crate::providers::revision(conn, id)?;
            }
            put(
                conn,
                &format!("{ENGINE_PROVIDER}{}", db::to_word(&backend)?),
                Some(&choice),
            )?;
            load(conn)
        })?;
        info!(backend = backend.label(), "engine provider set");
        Ok(prefs)
    }

    fn set_default_agent(&self, backend: Option<BackendKind>) -> Result<ModelPrefs> {
        let prefs = self.write(|conn| {
            put(conn, DEFAULT_AGENT, backend.as_ref())?;
            load(conn)
        })?;
        info!(
            backend = backend.map_or("none", BackendKind::label),
            "default agent set"
        );
        Ok(prefs)
    }

    fn set_hidden(&self, model: &ModelRef, hidden: bool) -> Result<ModelPrefs> {
        self.write(|conn| {
            let params = (model.provider.as_str(), model.model_id.as_str());
            if hidden {
                ensure_model(conn, model)?;
                conn.execute(
                    "INSERT OR IGNORE INTO hidden_models (provider_id, model_id) VALUES (?1, ?2)",
                    params,
                )?;
            } else {
                conn.execute(
                    "DELETE FROM hidden_models WHERE provider_id = ?1 AND model_id = ?2",
                    params,
                )?;
            }
            load(conn)
        })
    }

    fn set_context_overrides(&self, changes: &[(ModelRef, Option<u32>)]) -> Result<ModelPrefs> {
        if let Some(tokens) = changes
            .iter()
            .filter_map(|(_, tokens)| *tokens)
            .find(|tokens| !CONTEXT_OVERRIDE_RANGE.contains(tokens))
        {
            return Err(Error::invalid(format!(
                "上下文窗口 {tokens} 超出范围：只能是 {}–{} token。",
                CONTEXT_OVERRIDE_RANGE.start(),
                CONTEXT_OVERRIDE_RANGE.end()
            )));
        }
        let prefs = self.write(|conn| {
            for (model, tokens) in changes {
                if tokens.is_some() {
                    ensure_model(conn, model)?;
                }
                put(conn, &context_key(model), tokens.as_ref())?;
            }
            load(conn)
        })?;
        info!(changed = changes.len(), "context overrides set");
        Ok(prefs)
    }

    fn set_review_model(
        &self,
        model: Option<ModelRef>,
        effort: Option<ThinkingLevel>,
    ) -> Result<ModelPrefs> {
        let prefs = self.write(|conn| {
            if let Some(model) = &model {
                ensure_model(conn, model)?;
            }
            put(conn, REVIEW_MODEL, model.as_ref())?;
            put(conn, REVIEW_EFFORT, effort.as_ref())?;
            load(conn)
        })?;
        info!(
            provider = prefs.review.model.as_ref().map(|m| m.provider.as_str()),
            "review model set"
        );
        Ok(prefs)
    }

    fn set_review_switch(&self, switch: ReviewSwitch, on: bool) -> Result<ModelPrefs> {
        let key = match switch {
            ReviewSwitch::AgentReview => AGENT_REVIEW,
            ReviewSwitch::AutoFix => REVIEW_AUTO_FIX,
        };
        // Off is the default, so it is stored as an absent key.
        let prefs = self.write(|conn| {
            put(conn, key, on.then_some(&true))?;
            load(conn)
        })?;
        info!(switch = key, on, "review switch set");
        Ok(prefs)
    }
}
