//! `ProviderService`: providers, endpoints, models, keys and round-robin key selection
//! (agent-settings 04 §2.2, §3.2–§3.4).

use thinkrail_core::{
    Endpoint, Error, Provider, ProviderId, ProviderInput, ProviderKeyId, ProviderKeyInput, ProviderKeySummary,
    ProviderKeyUpdate, ProviderModel, ProviderService, Result, Secret, SelectedKey,
};
use tracing::info;
use turso::Row;

use crate::{
    db::{self, Conn},
    mask::mask,
    store::SqliteSettingsStore,
    validate,
};

const PROVIDER_COLUMNS: &str = "id, name, default_protocol, headers, access, enabled, revision";

fn provider_missing() -> Error {
    Error::not_found("提供方不存在，可能已被删除。")
}

fn key_missing() -> Error {
    Error::not_found("API 密钥不存在，可能已被删除。")
}

/// Current revision of a provider; `NotFound` when it is gone.
pub(crate) fn revision(conn: &Conn, id: &ProviderId) -> Result<u64> {
    let row = conn.row("SELECT revision FROM providers WHERE id = ?1", (id.as_str(),))?.ok_or_else(provider_missing)?;
    db::revision(&row, 0)
}

fn expect(conn: &Conn, id: &ProviderId, expect_revision: u64) -> Result<()> {
    if revision(conn, id)? != expect_revision {
        return Err(Error::conflict("提供方已被修改，请重新打开后再保存。"));
    }
    Ok(())
}

fn bump(conn: &Conn, id: &ProviderId) -> Result<()> {
    conn.execute("UPDATE providers SET revision = revision + 1, updated_at = ?2 WHERE id = ?1", (id.as_str(), db::now()))?;
    Ok(())
}

fn ensure_unique_name(conn: &Conn, name: &str, except: Option<&ProviderId>) -> Result<()> {
    let except = except.map_or("", ProviderId::as_str);
    if conn.row("SELECT 1 FROM providers WHERE name = ?1 AND id <> ?2", (name, except))?.is_some() {
        return Err(Error::conflict(format!("已有同名提供方 {name}。")));
    }
    Ok(())
}

fn ensure_unique_key(conn: &Conn, id: &ProviderId, key: &str, except: Option<&ProviderKeyId>) -> Result<()> {
    let except = except.map_or("", ProviderKeyId::as_str);
    let sql = "SELECT 1 FROM provider_keys WHERE provider_id = ?1 AND key = ?2 AND id <> ?3";
    if conn.row(sql, (id.as_str(), key, except))?.is_some() {
        return Err(Error::conflict("API 密钥已存在"));
    }
    Ok(())
}

/// Rewrite endpoints and models of `id` from `input` (rows must not exist yet).
fn write_children(conn: &Conn, id: &ProviderId, input: &ProviderInput) -> Result<()> {
    for endpoint in &input.endpoints {
        conn.execute(
            "INSERT INTO provider_endpoints (provider_id, protocol, base_url) VALUES (?1, ?2, ?3)",
            (id.as_str(), db::to_word(&endpoint.protocol)?, endpoint.base_url.as_str()),
        )?;
    }
    for (sort, model) in input.models.iter().enumerate() {
        conn.execute(
            "INSERT INTO provider_models (provider_id, model_id, name, protocols, context_window, max_tokens, \
             vision, reasoning, tools, efforts, enabled, sort) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            (
                id.as_str(),
                model.model_id.as_str(),
                model.name.as_str(),
                db::to_json(&model.protocols)?,
                model.context_window,
                model.max_tokens,
                model.vision,
                model.reasoning,
                model.tools,
                db::to_json(&model.efforts)?,
                model.enabled,
                i64::try_from(sort).unwrap_or(i64::MAX),
            ),
        )?;
    }
    Ok(())
}

fn decode(conn: &Conn, row: &Row) -> Result<Provider> {
    let id = ProviderId::from(db::text(row, 0)?);
    let mut endpoints = conn
        .rows("SELECT protocol, base_url FROM provider_endpoints WHERE provider_id = ?1", (id.as_str(),))?
        .iter()
        .map(|row| Ok(Endpoint { protocol: db::word(row, 0)?, base_url: db::text(row, 1)? }))
        .collect::<Result<Vec<_>>>()?;
    endpoints.sort_by_key(|endpoint| validate::protocol_rank(endpoint.protocol));
    let models = conn
        .rows(
            "SELECT model_id, name, protocols, context_window, max_tokens, vision, reasoning, tools, efforts, enabled \
             FROM provider_models WHERE provider_id = ?1 ORDER BY sort",
            (id.as_str(),),
        )?
        .iter()
        .map(|row| {
            Ok(ProviderModel {
                model_id: db::text(row, 0)?,
                name: db::text(row, 1)?,
                protocols: db::json(row, 2)?,
                context_window: db::optional_u32(row, 3)?,
                max_tokens: db::optional_u32(row, 4)?,
                vision: db::flag(row, 5)?,
                reasoning: db::flag(row, 6)?,
                tools: db::flag(row, 7)?,
                efforts: db::json(row, 8)?,
                enabled: db::flag(row, 9)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let keys = conn
        .rows("SELECT id, label, key, enabled FROM provider_keys WHERE provider_id = ?1 ORDER BY sort", (id.as_str(),))?
        .iter()
        .map(|row| {
            Ok(ProviderKeySummary {
                id: db::text(row, 0)?.into(),
                label: db::optional_text(row, 1)?,
                masked: mask(&db::text(row, 2)?),
                enabled: db::flag(row, 3)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Provider {
        revision: db::revision(row, 6)?,
        input: ProviderInput {
            name: db::text(row, 1)?,
            endpoints,
            default_protocol: db::word(row, 2)?,
            headers: db::json(row, 3)?,
            access: db::word(row, 4)?,
            enabled: db::flag(row, 5)?,
            models,
        },
        keys,
        id,
    })
}

fn load(conn: &Conn, id: &ProviderId) -> Result<Provider> {
    let sql = format!("SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = ?1");
    let row = conn.row(&sql, (id.as_str(),))?.ok_or_else(provider_missing)?;
    decode(conn, &row)
}

/// Enabled keys of a provider in rotation order: `(id, label, value)`.
fn enabled_keys(conn: &Conn, id: &ProviderId) -> Result<Vec<SelectedKey>> {
    conn.rows(
        "SELECT id, label, key FROM provider_keys WHERE provider_id = ?1 AND enabled = 1 ORDER BY sort",
        (id.as_str(),),
    )?
    .iter()
    .map(|row| {
        Ok(SelectedKey {
            id: db::text(row, 0)?.into(),
            label: db::optional_text(row, 1)?,
            value: Secret::new(db::text(row, 2)?),
        })
    })
    .collect()
}

impl ProviderService for SqliteSettingsStore {
    fn providers(&self) -> Result<Vec<Provider>> {
        self.read(|conn| {
            let sql = format!("SELECT {PROVIDER_COLUMNS} FROM providers ORDER BY sort, name");
            conn.rows(&sql, ())?.iter().map(|row| decode(conn, row)).collect()
        })
    }

    fn provider(&self, id: &ProviderId) -> Result<Provider> {
        self.read(|conn| load(conn, id))
    }

    fn create(&self, input: ProviderInput) -> Result<Provider> {
        let input = validate::provider(input)?;
        let id = ProviderId::generate();
        let provider = self.write(|conn| {
            ensure_unique_name(conn, &input.name, None)?;
            let sort = conn.int("SELECT COALESCE(MAX(sort), 0) + 1 FROM providers", ())?;
            let now = db::now();
            conn.execute(
                "INSERT INTO providers (id, name, default_protocol, headers, access, enabled, sort, revision, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
                (
                    id.as_str(),
                    input.name.as_str(),
                    db::to_word(&input.default_protocol)?,
                    db::to_json(&input.headers)?,
                    db::to_word(&input.access)?,
                    input.enabled,
                    sort,
                    now,
                ),
            )?;
            write_children(conn, &id, &input)?;
            load(conn, &id)
        })?;
        info!(provider = %provider.id, "provider created");
        Ok(provider)
    }

    fn update(&self, id: &ProviderId, expect_revision: u64, input: ProviderInput) -> Result<Provider> {
        let input = validate::provider(input)?;
        let provider = self.write(|conn| {
            expect(conn, id, expect_revision)?;
            ensure_unique_name(conn, &input.name, Some(id))?;
            conn.execute(
                "UPDATE providers SET name = ?2, default_protocol = ?3, headers = ?4, access = ?5, enabled = ?6, \
                 revision = revision + 1, updated_at = ?7 WHERE id = ?1",
                (
                    id.as_str(),
                    input.name.as_str(),
                    db::to_word(&input.default_protocol)?,
                    db::to_json(&input.headers)?,
                    db::to_word(&input.access)?,
                    input.enabled,
                    db::now(),
                ),
            )?;
            conn.execute("DELETE FROM provider_endpoints WHERE provider_id = ?1", (id.as_str(),))?;
            conn.execute("DELETE FROM provider_models WHERE provider_id = ?1", (id.as_str(),))?;
            write_children(conn, id, &input)?;
            conn.execute(
                "DELETE FROM hidden_models WHERE provider_id = ?1 \
                 AND model_id NOT IN (SELECT model_id FROM provider_models WHERE provider_id = ?1)",
                (id.as_str(),),
            )?;
            load(conn, id)
        })?;
        info!(provider = %id, revision = provider.revision, "provider updated");
        Ok(provider)
    }

    fn delete(&self, id: &ProviderId, expect_revision: u64) -> Result<()> {
        self.write(|conn| {
            expect(conn, id, expect_revision)?;
            // Endpoints, models, keys and hidden-model rows go with it through ON DELETE CASCADE.
            conn.execute("DELETE FROM providers WHERE id = ?1", (id.as_str(),))?;
            Ok(())
        })?;
        self.cursor.lock().remove(id);
        info!(provider = %id, "provider deleted");
        Ok(())
    }

    fn add_key(&self, id: &ProviderId, key: ProviderKeyInput) -> Result<Provider> {
        let value = validate::key(&key.key)?;
        let label = validate::label(key.label)?;
        let key_id = ProviderKeyId::generate();
        let provider = self.write(|conn| {
            revision(conn, id)?;
            ensure_unique_key(conn, id, &value, None)?;
            let sort = conn.int("SELECT COALESCE(MAX(sort), 0) + 1 FROM provider_keys WHERE provider_id = ?1", (id.as_str(),))?;
            conn.execute(
                "INSERT INTO provider_keys (id, provider_id, key, label, enabled, sort) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (key_id.as_str(), id.as_str(), value.as_str(), label.as_deref(), key.enabled, sort),
            )?;
            bump(conn, id)?;
            load(conn, id)
        })?;
        info!(provider = %id, key = %key_id, "provider key added");
        Ok(provider)
    }

    fn update_key(&self, id: &ProviderId, key_id: &ProviderKeyId, key: ProviderKeyUpdate) -> Result<Provider> {
        let value = key.key.as_ref().map(validate::key).transpose()?;
        let label = validate::label(key.label)?;
        let provider = self.write(|conn| {
            revision(conn, id)?;
            let sql = "SELECT 1 FROM provider_keys WHERE id = ?1 AND provider_id = ?2";
            conn.row(sql, (key_id.as_str(), id.as_str()))?.ok_or_else(key_missing)?;
            conn.execute(
                "UPDATE provider_keys SET label = ?2, enabled = ?3 WHERE id = ?1",
                (key_id.as_str(), label.as_deref(), key.enabled),
            )?;
            if let Some(value) = &value {
                ensure_unique_key(conn, id, value, Some(key_id))?;
                conn.execute("UPDATE provider_keys SET key = ?2 WHERE id = ?1", (key_id.as_str(), value.as_str()))?;
            }
            bump(conn, id)?;
            load(conn, id)
        })?;
        info!(provider = %id, key = %key_id, "provider key updated");
        Ok(provider)
    }

    fn delete_key(&self, id: &ProviderId, key_id: &ProviderKeyId) -> Result<Provider> {
        let provider = self.write(|conn| {
            revision(conn, id)?;
            let sql = "DELETE FROM provider_keys WHERE id = ?1 AND provider_id = ?2";
            if conn.execute(sql, (key_id.as_str(), id.as_str()))? == 0 {
                return Err(key_missing());
            }
            bump(conn, id)?;
            load(conn, id)
        })?;
        info!(provider = %id, key = %key_id, "provider key deleted");
        Ok(provider)
    }

    fn reveal_key(&self, key_id: &ProviderKeyId) -> Result<Secret> {
        self.read(|conn| {
            let row = conn.row("SELECT key FROM provider_keys WHERE id = ?1", (key_id.as_str(),))?.ok_or_else(key_missing)?;
            Ok(Secret::new(db::text(&row, 0)?))
        })
    }

    /// 04 §3.4: first key when there is no cursor or it points at a key no longer enabled,
    /// otherwise the one after it, wrapping around.
    fn next_key(&self, id: &ProviderId) -> Result<Option<SelectedKey>> {
        let mut keys = self.read(|conn| {
            revision(conn, id)?;
            enabled_keys(conn, id)
        })?;
        if keys.len() <= 1 {
            return Ok(keys.pop());
        }
        let mut cursor = self.cursor.lock();
        let position = cursor
            .get(id)
            .and_then(|last| keys.iter().position(|key| &key.id == last))
            .map_or(0, |last| (last + 1) % keys.len());
        let selected = keys.swap_remove(position);
        cursor.insert(id.clone(), selected.id.clone());
        Ok(Some(selected))
    }
}
