//! `ChatBindingService`: which engine each chat runs and its native session (agent-settings 07 §4).

use thinkrail_core::{ChatBinding, ChatBindingService, ChatId, Result, WorkspaceId};
use tracing::debug;

use crate::{db, store::SqliteSettingsStore};

impl ChatBindingService for SqliteSettingsStore {
    fn bind(&self, binding: ChatBinding) -> Result<()> {
        let backend = db::to_word(&binding.backend)?;
        self.write(|conn| {
            conn.execute(
                "INSERT INTO chat_bindings (chat_id, workspace_id, backend, native_session_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (chat_id) DO UPDATE SET backend = excluded.backend, native_session_key = excluded.native_session_key",
                (binding.chat_id.as_str(), binding.workspace_id.as_str(), backend.as_str(), binding.native_session_key.as_str(), db::now()),
            )
        })?;
        debug!(chat = %binding.chat_id, backend = binding.backend.label(), "settings.chat.bound");
        Ok(())
    }

    fn bindings(&self, workspace: &WorkspaceId) -> Result<Vec<ChatBinding>> {
        self.read(|conn| {
            let sql = "SELECT chat_id, backend, native_session_key FROM chat_bindings WHERE workspace_id = ?1 ORDER BY created_at, rowid";
            conn.rows(sql, (workspace.as_str(),))?
                .iter()
                .map(|row| {
                    Ok(ChatBinding {
                        chat_id: db::text(row, 0)?.into(),
                        workspace_id: workspace.clone(),
                        backend: db::word(row, 1)?,
                        native_session_key: db::text(row, 2)?.into(),
                    })
                })
                .collect()
        })
    }

    fn unbind(&self, chat: &ChatId) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "DELETE FROM chat_bindings WHERE chat_id = ?1",
                (chat.as_str(),),
            )
        })?;
        Ok(())
    }
}
