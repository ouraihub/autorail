//! Unit tests for MCP form validation and JSON import.

use super::*;
use super::{
    form::{Form, to_input},
    import::parse_import,
};
use thinkrail_core::{EnvValue, McpTransport};

fn scope() -> McpScope {
    McpScope::Global
}
fn form() -> Form {
    Form {
        name: "demo_server".into(),
        transport: TransportKind::Stdio,
        command: "node".into(),
        args: "server.js\n--stdio".into(),
        url: String::new(),
        env: vec![EnvValue {
            name: "TOKEN".into(),
            value: "$TOKEN".into(),
        }],
        headers: Vec::new(),
        description: String::new(),
        enabled: true,
        scope: scope(),
    }
}

#[test]
fn structured_form_preserves_argument_order_and_environment_references() {
    let input = to_input(&form()).unwrap();
    let McpTransport::Stdio { command, args, env } = input.transport else {
        panic!()
    };
    assert_eq!(command, "node");
    assert_eq!(args, ["server.js", "--stdio"]);
    assert_eq!(
        env,
        [EnvValue {
            name: "TOKEN".into(),
            value: "$TOKEN".into()
        }]
    );
}

#[test]
fn rejects_invalid_name_and_pairs() {
    assert_eq!(
        to_input(&Form {
            name: "bad name".into(),
            ..form()
        }),
        Err(s::NAME_INVALID)
    );
    assert_eq!(
        to_input(&Form {
            env: vec![EnvValue {
                name: " ".into(),
                value: "secret".into()
            }],
            ..form()
        }),
        Err(s::ENV_INVALID)
    );
}

#[test]
fn import_keeps_only_valid_items() {
    let raw = r#"{"mcpServers":{"ok":{"command":"node","args":[],"env":{}},"bad name":{"command":"node","args":[],"env":{}}}}"#;
    let preview = parse_import(raw, scope());
    assert_eq!(preview.as_ref().map(|p| p.valid.len()), Ok(1));
    assert_eq!(preview.as_ref().map(|p| p.invalid), Ok(1));
}

#[test]
fn import_forces_current_scope_and_keeps_disabled_servers() {
    let scope = McpScope::Project {
        project: "current".into(),
    };
    let preview = parse_import(r#"{"local":{"command":"node","disabled":true,"scope":"global"},"bad-url":{"url":"https://user:password@example.com/mcp"},"good-url":{"url":"https://example.com/mcp"}}"#, scope.clone()).unwrap();
    assert_eq!(preview.valid.len(), 2);
    assert_eq!(preview.invalid, 1);
    assert!(preview.valid.iter().all(|input| input.scope == scope));
    let local = preview
        .valid
        .iter()
        .find(|input| input.name == "local")
        .unwrap();
    assert!(!local.enabled);
    assert!(matches!(&local.transport, McpTransport::Stdio { args, .. } if args.is_empty()));
    assert_eq!(
        parse_import(r#"{"mcpServers":[]}"#, scope),
        Err(s::IMPORT_INVALID)
    );
}

#[gpui_kit::test]
fn pair_add_buttons_name_what_they_add(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (fixture, page) = crate::testing::open(cx, McpPage::new);
    let label = |cx: &mut gpui_kit::TestAppContext, id: String| {
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().map(str::to_owned)
        })
        .unwrap()
    };

    cx.update_window(fixture.window, |_, window, cx| {
        page.update(cx, |page, cx| page.begin_edit(None, window, cx));
    })
    .unwrap();
    assert_eq!(
        label(cx, format!("mcp-add-{}", s::ENV)).as_deref(),
        Some(s::ADD_ENV)
    );

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            if let Some(editor) = &mut page.editor {
                editor.transport = TransportKind::Http;
            }
            cx.notify();
        });
    });
    assert_eq!(
        label(cx, format!("mcp-add-{}", s::HEADERS)).as_deref(),
        Some(s::ADD_HEADER)
    );
}
