//! MCP JSON import: parsing pasted configuration into a preview and creating its servers.

use gpui_kit::Context;
use serde_json::Value;
use thinkrail_core::{EnvValue, McpScope, McpServerInput, McpTransport};

use super::{
    ImportPreview, McpPage, ViewScope,
    form::{valid_name, validate_url},
};
use crate::{
    state::{AppEvent, run},
    strings::settings_mcp as s,
};

fn import_items(value: Value) -> Result<Vec<(String, Value)>, &'static str> {
    let object = value.as_object().ok_or(s::IMPORT_INVALID)?;
    let source = match object.get("mcpServers") {
        Some(servers) => servers.as_object().ok_or(s::IMPORT_INVALID)?,
        None => object,
    };
    if source.is_empty() {
        return Err(s::IMPORT_EMPTY);
    }
    Ok(source
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect())
}

fn import_transport(value: &Value) -> Result<McpTransport, &'static str> {
    let object = value.as_object().ok_or(s::IMPORT_ITEM_INVALID)?;
    if let Some(command) = object.get("command").and_then(Value::as_str) {
        let command = command.trim();
        if command.is_empty() {
            return Err(s::COMMAND_REQUIRED);
        }
        let args = object.get("args").map_or_else(
            || Ok(Vec::new()),
            |value| {
                value
                    .as_array()
                    .ok_or(s::IMPORT_ITEM_INVALID)
                    .and_then(|args| {
                        args.iter()
                            .map(|arg| arg.as_str().map(ToOwned::to_owned))
                            .collect::<Option<Vec<_>>>()
                            .ok_or(s::IMPORT_ITEM_INVALID)
                    })
            },
        )?;
        let env = object
            .get("env")
            .map(|value| parse_import_pairs(value, s::ENV_INVALID))
            .transpose()?
            .unwrap_or_default();
        return Ok(McpTransport::Stdio {
            command: command.to_owned(),
            args,
            env,
        });
    }
    if let Some(url) = object.get("url").and_then(Value::as_str) {
        validate_url(url)?;
        let headers = object
            .get("headers")
            .map(|value| parse_import_pairs(value, s::HEADERS_INVALID))
            .transpose()?
            .unwrap_or_default();
        return Ok(McpTransport::Http {
            url: url.to_owned(),
            headers,
        });
    }
    Err(s::IMPORT_ITEM_INVALID)
}

fn parse_import_pairs(value: &Value, error: &'static str) -> Result<Vec<EnvValue>, &'static str> {
    let object = value.as_object().ok_or(error)?;
    object
        .iter()
        .map(|(name, value)| {
            let name = name.trim();
            if name.is_empty() {
                return Err(error);
            };
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(|value| EnvValue {
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
                .ok_or(error)
        })
        .collect()
}

pub(super) fn parse_import(raw: &str, scope: McpScope) -> Result<ImportPreview, &'static str> {
    let value: Value = serde_json::from_str(raw).map_err(|_| s::IMPORT_INVALID)?;
    let items = import_items(value)?;
    let mut valid = Vec::new();
    let mut invalid = 0;
    for (name, value) in items {
        if !valid_name(&name) || name.len() > 32 {
            invalid += 1;
            continue;
        }
        let transport = match import_transport(&value) {
            Ok(transport) => transport,
            Err(_) => {
                invalid += 1;
                continue;
            }
        };
        let object = value.as_object().ok_or(s::IMPORT_ITEM_INVALID)?;
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let enabled = object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && !object
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        valid.push(McpServerInput {
            name,
            scope: scope.clone(),
            transport,
            enabled,
            description,
        });
    }
    if valid.is_empty() && invalid == 0 {
        return Err(s::IMPORT_EMPTY);
    }
    Ok(ImportPreview { valid, invalid })
}

impl McpPage {
    pub(super) fn preview_import(&mut self, cx: &mut Context<Self>) {
        let raw = self.import.read(cx).value().to_string();
        let scope = match self.scope {
            ViewScope::Global => McpScope::Global,
            ViewScope::Project => self
                .project(cx)
                .map_or(McpScope::Global, |project| McpScope::Project { project }),
        };
        match parse_import(&raw, scope) {
            Ok(preview) => {
                self.error = None;
                self.import_preview = Some(preview);
            }
            Err(error) => {
                self.error = Some(error);
                self.import_preview = None;
            }
        }
        cx.notify();
    }

    pub(super) fn import_valid(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.import_preview.clone() else {
            return;
        };
        if self.importing || preview.valid.is_empty() {
            return;
        }
        self.importing = true;
        let pending = self.import.clone();
        let service = self.state.read(cx).services.mcp.clone();
        run(
            cx,
            move || {
                let mut succeeded = 0usize;
                let mut failure = None;
                for input in preview.valid {
                    match service.create(input) {
                        Ok(_) => succeeded += 1,
                        Err(error) => failure = Some(error),
                    }
                }
                Ok((succeeded, failure))
            },
            move |this, result, cx| {
                this.importing = false;
                if let Ok((succeeded, failure)) = result {
                    if this.import == pending {
                        this.import_preview = None;
                    }
                    if succeeded > 0 {
                        this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                        this.reload(cx);
                    }
                    if let Some(error) = failure {
                        this.state.update(cx, |state, cx| state.report(&error, cx));
                    }
                }
                cx.notify();
            },
        );
    }
}
