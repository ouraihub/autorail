//! `McpService`: global and project MCP servers, project overrides and the effective set
//! (agent-settings 04 §2.3, 02 §3.2).

use std::collections::{HashMap, HashSet};

use thinkrail_core::{
    EffectiveMcpServer, Error, McpOrigin, McpScope, McpServer, McpServerId, McpServerInput, McpService, ProjectId,
    Result,
};
use tracing::info;
use turso::Row;

use crate::{
    db::{self, Conn},
    store::SqliteSettingsStore,
    validate,
};

const COLUMNS: &str = "id, name, project_id, transport, enabled, description, revision";

/// `project_id` column value: `''` for global entries.
fn scope_key(scope: &McpScope) -> &str {
    match scope {
        McpScope::Global => "",
        McpScope::Project { project } => project.as_str(),
    }
}

fn missing() -> Error {
    Error::not_found("MCP 服务器不存在，可能已被删除。")
}

fn decode(row: &Row) -> Result<McpServer> {
    let project = db::text(row, 2)?;
    let scope = if project.is_empty() { McpScope::Global } else { McpScope::Project { project: project.into() } };
    Ok(McpServer {
        id: db::text(row, 0)?.into(),
        revision: db::revision(row, 6)?,
        input: McpServerInput {
            name: db::text(row, 1)?,
            scope,
            transport: db::json(row, 3)?,
            enabled: db::flag(row, 4)?,
            description: db::optional_text(row, 5)?,
        },
    })
}

fn list(conn: &Conn, project: &str) -> Result<Vec<McpServer>> {
    let sql = format!("SELECT {COLUMNS} FROM mcp_servers WHERE project_id = ?1 ORDER BY sort, name");
    conn.rows(&sql, (project,))?.iter().map(decode).collect()
}

fn load(conn: &Conn, id: &McpServerId) -> Result<McpServer> {
    let sql = format!("SELECT {COLUMNS} FROM mcp_servers WHERE id = ?1");
    decode(&conn.row(&sql, (id.as_str(),))?.ok_or_else(missing)?)
}

fn ensure_unique_name(conn: &Conn, input: &McpServerInput, except: Option<&McpServerId>) -> Result<()> {
    let except = except.map_or("", McpServerId::as_str);
    let sql = "SELECT 1 FROM mcp_servers WHERE project_id = ?1 AND name = ?2 AND id <> ?3";
    if conn.row(sql, (scope_key(&input.scope), input.name.as_str(), except))?.is_some() {
        return Err(Error::conflict(format!("同一范围内已有名为 {} 的 MCP 服务器。", input.name)));
    }
    Ok(())
}

impl McpService for SqliteSettingsStore {
    fn global(&self) -> Result<Vec<McpServer>> {
        self.read(|conn| list(conn, ""))
    }

    /// Global entries first, then the project's own, each in `sort, name` order. A global entry
    /// whose name a project entry reuses stays in place as `GlobalOverridden` and is disabled.
    fn effective(&self, project: &ProjectId) -> Result<Vec<EffectiveMcpServer>> {
        validate::project(project)?;
        let (global, local, overrides) = self.read(|conn| {
            let overrides = conn
                .rows("SELECT server_id, enabled FROM mcp_overrides WHERE project_id = ?1", (project.as_str(),))?
                .iter()
                .map(|row| Ok((McpServerId::from(db::text(row, 0)?), db::flag(row, 1)?)))
                .collect::<Result<HashMap<_, _>>>()?;
            Ok((list(conn, "")?, list(conn, project.as_str())?, overrides))
        })?;
        let local_names: HashSet<&str> = local.iter().map(|server| server.input.name.as_str()).collect();
        let mut effective = Vec::with_capacity(global.len() + local.len());
        for server in global {
            let (origin, enabled) = if local_names.contains(server.input.name.as_str()) {
                (McpOrigin::GlobalOverridden, false)
            } else {
                (McpOrigin::Global, overrides.get(&server.id).copied().unwrap_or(server.input.enabled))
            };
            effective.push(EffectiveMcpServer { server, origin, enabled });
        }
        for server in local {
            let enabled = server.input.enabled;
            effective.push(EffectiveMcpServer { server, origin: McpOrigin::Project, enabled });
        }
        Ok(effective)
    }

    fn create(&self, input: McpServerInput) -> Result<McpServer> {
        let input = validate::mcp(input)?;
        let id = McpServerId::generate();
        let server = self.write(|conn| {
            ensure_unique_name(conn, &input, None)?;
            let sort = conn.int("SELECT COALESCE(MAX(sort), 0) + 1 FROM mcp_servers", ())?;
            conn.execute(
                "INSERT INTO mcp_servers (id, name, project_id, transport, enabled, description, revision, sort) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
                (
                    id.as_str(),
                    input.name.as_str(),
                    scope_key(&input.scope),
                    db::to_json(&input.transport)?,
                    input.enabled,
                    input.description.as_deref(),
                    sort,
                ),
            )?;
            load(conn, &id)
        })?;
        info!(server = %server.id, "mcp server created");
        Ok(server)
    }

    fn update(&self, id: &McpServerId, expect_revision: u64, input: McpServerInput) -> Result<McpServer> {
        let input = validate::mcp(input)?;
        let server = self.write(|conn| {
            if load(conn, id)?.revision != expect_revision {
                return Err(Error::conflict("MCP 服务器已被修改，请重新打开后再保存。"));
            }
            ensure_unique_name(conn, &input, Some(id))?;
            conn.execute(
                "UPDATE mcp_servers SET name = ?2, project_id = ?3, transport = ?4, enabled = ?5, description = ?6, \
                 revision = revision + 1 WHERE id = ?1",
                (
                    id.as_str(),
                    input.name.as_str(),
                    scope_key(&input.scope),
                    db::to_json(&input.transport)?,
                    input.enabled,
                    input.description.as_deref(),
                ),
            )?;
            if matches!(input.scope, McpScope::Project { .. }) {
                // Overrides only ever point at global entries.
                conn.execute("DELETE FROM mcp_overrides WHERE server_id = ?1", (id.as_str(),))?;
            }
            load(conn, id)
        })?;
        info!(server = %id, revision = server.revision, "mcp server updated");
        Ok(server)
    }

    fn delete(&self, id: &McpServerId, expect_revision: u64) -> Result<()> {
        self.write(|conn| {
            if load(conn, id)?.revision != expect_revision {
                return Err(Error::conflict("MCP 服务器已被修改，请重新打开后再删除。"));
            }
            // Project overrides go with it through ON DELETE CASCADE.
            conn.execute("DELETE FROM mcp_servers WHERE id = ?1", (id.as_str(),))?;
            Ok(())
        })?;
        info!(server = %id, "mcp server deleted");
        Ok(())
    }

    fn set_override(&self, project: &ProjectId, server: &McpServerId, enabled: Option<bool>) -> Result<()> {
        validate::project(project)?;
        self.write(|conn| {
            if load(conn, server)?.input.scope != McpScope::Global {
                return Err(Error::invalid("只能在项目里启停全局 MCP 服务器；项目级服务器请直接编辑。"));
            }
            conn.execute(
                "DELETE FROM mcp_overrides WHERE project_id = ?1 AND server_id = ?2",
                (project.as_str(), server.as_str()),
            )?;
            if let Some(enabled) = enabled {
                conn.execute(
                    "INSERT INTO mcp_overrides (project_id, server_id, enabled) VALUES (?1, ?2, ?3)",
                    (project.as_str(), server.as_str(), enabled),
                )?;
            }
            Ok(())
        })?;
        info!(server = %server, "mcp project override set");
        Ok(())
    }
}
