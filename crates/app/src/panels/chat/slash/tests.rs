//! The `/` menu in a headless window: rows appear for `/`, ↓ + Enter completes the draft without
//! sending it, a click picks a row, Esc hides the menu; tab titles prefer the session name.

use std::sync::Arc;

use gpui_kit::{AppContext as _, Entity, TestAppContext, test::TestWindowExt as _};
use thinkrail_core::{
    BackendKind, Capabilities, ChatBinding, ChatId, CommandScope, CommandSource, NativeSessionKey,
    SessionEvent, SessionPhase, SessionSink, SessionSnapshot, SlashCommand, WorkspaceId,
};

use super::super::{Chat, ChatPanel};
use crate::testing::{self, Fixture};

/// Engine events are irrelevant here: no engine runs.
struct Discard;

impl SessionSink for Discard {
    fn publish(&self, _: SessionEvent) {}
}

fn engine_command(name: &str) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: Some(format!("{name} 说明")),
        hint: None,
        source: CommandSource::Extension,
        scope: Some(CommandScope::Project),
    }
}

fn chat(id: &ChatId, session_name: Option<&str>) -> Chat {
    let workspace = WorkspaceId::from("ws");
    Chat {
        workspace: workspace.clone(),
        binding: Some(ChatBinding {
            chat_id: id.clone(),
            workspace_id: workspace,
            backend: BackendKind::Pi,
            native_session_key: NativeSessionKey::from("native"),
        }),
        session: None,
        snapshot: Some(SessionSnapshot {
            id: id.clone(),
            backend: BackendKind::Pi,
            native_session_key: None,
            cwd: std::env::temp_dir(),
            phase: SessionPhase::Closed,
            model: None,
            messages: Vec::new(),
            interactions: Vec::new(),
            capabilities: Capabilities {
                compact: true,
                rename: true,
                model_switch: true,
                ..Capabilities::default()
            },
            output_truncated: false,
            last_error: None,
            commands: vec![engine_command("review"), engine_command("skill:plan")],
            session_name: session_name.map(str::to_owned),
            revision: 1,
        }),
        usage: super::super::usage::Usage::default(),
        routing_signature: None,
        host_tool_options: Default::default(),
        catalog: Vec::new(),
        restart_pending: false,
        closing: false,
        starting: false,
        blocked: None,
        ordinal: 1,
        review_cards: std::collections::HashMap::new(),
        plan: None,
    }
}

fn open_chat(cx: &mut TestAppContext) -> (Fixture, Entity<ChatPanel>, ChatId) {
    let (fixture, panel) = testing::open(cx, |state, window, cx| {
        ChatPanel::new(state, Arc::new(Discard), window, cx)
    });
    let id = ChatId::from("chat-1");
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.workspace = Some(WorkspaceId::from("ws"));
            panel.chats.insert(id.clone(), chat(&id, None));
            panel.order = vec![id.clone()];
            panel.active = Some(id.clone());
            cx.notify();
        });
    });
    (fixture, panel, id)
}

/// Clears the draft and focuses the composer, then types `text` as the user would.
fn type_fresh(cx: &mut TestAppContext, fixture: &Fixture, panel: &Entity<ChatPanel>, text: &str) {
    cx.update_window(fixture.window, |_, window, cx| {
        let composer = panel.read(cx).composer.clone();
        composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        window.render_frame(cx);
        window.input(text, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn draft(cx: &mut TestAppContext, panel: &Entity<ChatPanel>) -> String {
    cx.update(|cx| panel.read(cx).composer.read(cx).value().to_string())
}

fn row_count(cx: &mut TestAppContext, fixture: &Fixture) -> usize {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        (0..)
            .take_while(|i| window.try_find(format!("slash-row-{i}")).is_some())
            .count()
    })
    .unwrap()
}

#[gpui_kit::test]
fn slash_menu_completes_without_sending_and_hides_on_escape(cx: &mut TestAppContext) {
    let (fixture, panel, id) = open_chat(cx);

    type_fresh(cx, &fixture, &panel, "/");
    // compact, name, model (host) + review, skill:plan (engine).
    assert_eq!(row_count(cx, &fixture), 5);

    cx.update_window(fixture.window, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(draft(cx, &panel), "/name ");
    assert_eq!(row_count(cx, &fixture), 0);
    cx.update(|cx| {
        let chat = &panel.read(cx).chats[&id];
        assert!(!chat.starting && chat.session.is_none());
    });

    // Up from the first row wraps to the last; Tab selects too.
    type_fresh(cx, &fixture, &panel, "/");
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("up", cx);
        window.press("tab", cx);
    })
    .unwrap();
    assert_eq!(draft(cx, &panel), "/skill:plan ");

    // Filtering narrows the rows; a click picks one.
    type_fresh(cx, &fixture, &panel, "/MO");
    assert_eq!(row_count(cx, &fixture), 1);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("slash-row-0", cx);
    })
    .unwrap();
    assert_eq!(draft(cx, &panel), "/model ");

    // Esc hides the menu and leaves the draft; the next edit shows it again.
    type_fresh(cx, &fixture, &panel, "/");
    cx.update_window(fixture.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert_eq!(row_count(cx, &fixture), 0);
    assert_eq!(draft(cx, &panel), "/");
    cx.update_window(fixture.window, |_, window, cx| window.input("r", cx))
        .unwrap();
    assert_eq!(row_count(cx, &fixture), 1);
}

#[test]
fn tab_title_prefers_the_session_name() {
    let id = ChatId::from("chat-1");
    assert_eq!(chat(&id, Some("修复登录")).title(), "修复登录");
    assert_eq!(chat(&id, Some("  ")).title(), "聊天 1");
    assert_eq!(chat(&id, None).title(), "聊天 1");
}
