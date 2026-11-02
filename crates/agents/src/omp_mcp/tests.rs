use std::{fs, path::Path};

use serde_json::{Value, json};
use thinkrail_core::ErrorKind;

use super::*;
use crate::testing::{SECRETS, sample_routing};

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn path() -> &'static Path {
    Path::new("config/mcp.json")
}

#[test]
fn merge_keeps_user_entries_and_unknown_fields_and_drops_stale_host_entries() {
    let current = json!({
        "mcpServers": {
            "user-tool": {"command": "uvx", "args": ["tool"], "timeout": 5000},
            "thinkrail-gone": {"command": "old"},
        },
        "disabledServers": ["user-other"],
        "futureField": {"x": 1},
    })
    .to_string();
    let servers = sample_routing().mcp;
    let merged = parse(&merge(path(), Some(&current), &servers).unwrap().unwrap());
    assert_eq!(
        merged,
        json!({
            "mcpServers": {
                "user-tool": {"command": "uvx", "args": ["tool"], "timeout": 5000},
                "thinkrail-echo": {"type": "stdio", "command": "node", "args": ["echo.js"],
                                   "env": {"ECHO_TOKEN": "${THINKRAIL_MCP_5E6F7A8B_E0}"}},
                "thinkrail-docs": {"type": "http", "url": "https://mcp.example.com/mcp",
                                   "headers": {"Authorization": "${THINKRAIL_MCP_0A0B0C0D_H0}"}},
            },
            "disabledServers": ["user-other"],
            "futureField": {"x": 1},
        })
    );
    let emptied = parse(&merge(path(), Some(&current), &[]).unwrap().unwrap());
    assert_eq!(
        emptied["mcpServers"],
        json!({"user-tool": {"command": "uvx", "args": ["tool"], "timeout": 5000}})
    );
}

#[test]
fn merge_reports_no_change_when_the_content_is_the_same() {
    let servers = sample_routing().mcp;
    let written = merge(path(), None, &servers).unwrap().unwrap();
    assert_eq!(merge(path(), Some(&written), &servers).unwrap(), None);
    assert_eq!(
        merge(path(), None, &[]).unwrap(),
        None,
        "no file and nothing to project stays without a file"
    );
    assert_eq!(
        merge(path(), Some(r#"{"other": true}"#), &[]).unwrap(),
        None
    );
}

#[test]
fn merge_refuses_invalid_json_and_non_object_servers() {
    let servers = sample_routing().mcp;
    for text in ["{ not json", "[1, 2]"] {
        let error = merge(path(), Some(text), &servers).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::Invalid));
        assert!(error.to_string().contains("不是有效的 JSON"), "{error}");
    }
    let error = merge(path(), Some(r#"{"mcpServers": []}"#), &servers).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
}

#[test]
fn sync_writes_references_never_values_and_leaves_invalid_files_alone() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("mcp.json");
    let servers = sample_routing().mcp;
    sync(root.path(), &servers).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    for secret in SECRETS {
        assert!(!text.contains(secret), "mcp.json contains {secret}");
    }
    assert!(text.contains("${THINKRAIL_MCP_5E6F7A8B_E0}"));

    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    sync(root.path(), &servers).unwrap();
    assert_eq!(
        fs::metadata(&file).unwrap().modified().unwrap(),
        modified,
        "unchanged content is not rewritten"
    );

    fs::write(&file, "{ broken").unwrap();
    let error = sync(root.path(), &servers).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert_eq!(fs::read_to_string(&file).unwrap(), "{ broken");
    sync(root.path(), &[]).unwrap();
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "{ broken",
        "nothing to project: the launch goes on untouched"
    );
    let leftovers: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["mcp.json"]);
}
