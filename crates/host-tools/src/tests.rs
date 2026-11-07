use std::{fs, io::Cursor, path::PathBuf, sync::Arc};

use serde_json::{Value, json};
use tempfile::TempDir;
use thinkrail_core::{
    HostToolHandler, HostToolOptions, HostToolResult, HostToolSpec, NewReviewComment,
    PlanReviewService, PlanScope, PlanStatus, Result, ReviewState, WorkspaceId, WorkspaceRoots,
};
use thinkrail_plan_review::JsonPlanReviewStore;

use crate::{HostTools, ToolBridge, bridge::BridgeClient, mcp};

struct Root(PathBuf);

impl WorkspaceRoots for Root {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

struct Fixture {
    _dirs: (TempDir, TempDir),
    store: Arc<JsonPlanReviewStore>,
    tools: HostTools,
    scope: PlanScope,
}

fn fixture() -> Fixture {
    let (data, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fs::write(work.path().join("a.txt"), "one\ntwo\n").unwrap();
    let root: Arc<dyn WorkspaceRoots> = Arc::new(Root(work.path().to_path_buf()));
    let store = Arc::new(JsonPlanReviewStore::open(data.path(), root.clone()).unwrap());
    let tools = HostTools::new(store.clone(), root);
    Fixture {
        _dirs: (data, work),
        store,
        tools,
        scope: PlanScope {
            workspace: "w".into(),
            chat: "c".into(),
        },
    }
}

impl Fixture {
    fn call(&self, name: &str, args: Value) -> HostToolResult {
        self.tools
            .call(&self.scope, HostToolOptions::default(), name, args)
            .unwrap()
    }

    fn ok(&self, name: &str, args: Value) -> String {
        let result = self.call(name, args);
        assert!(!result.is_error, "{name} failed: {}", result.text);
        result.text
    }

    fn id_of(&self, title: &str) -> String {
        let plan = self.store.plan(&self.scope).unwrap();
        thinkrail_plan_review::plan_ops::items(&plan)
            .find(|i| i.title == title)
            .unwrap()
            .id
            .to_string()
    }
}

#[test]
fn specs_are_the_original_tools_plus_resolve_comment() {
    let names: Vec<String> = fixture()
        .tools
        .specs(HostToolOptions::default())
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(
        names,
        [
            "todo_list",
            "todo_add",
            "todo_update",
            "todo_promote",
            "todo_remove",
            "todo_write",
            "todo_plan_summary",
            "spec_grep",
            "spec_get",
            "spec_graph",
            "spec_create",
            "spec_update",
            "spec_delete",
            "spec_validate",
            "resolve_comment"
        ]
    );
}

#[test]
fn add_list_update_and_completion_semantics() {
    let f = fixture();
    assert_eq!(f.ok("todo_list", json!({})), "The plan is empty.");
    assert!(
        f.ok(
            "todo_add",
            json!({"title": "Write parser", "group": "Parser"})
        )
        .starts_with("Added: [ ] Write parser — ")
    );
    f.ok(
        "todo_add",
        json!({"title": "Test parser", "group": "Parser"}),
    );
    let (first, second) = (f.id_of("Write parser"), f.id_of("Test parser"));
    let listed = f.ok("todo_list", json!({}));
    assert!(listed.starts_with("▸ Parser [pending 0/2]"), "{listed}");
    assert!(listed.contains("note: nothing is in_progress"), "{listed}");

    f.ok("todo_update", json!({"id": first, "status": "in_progress"}));
    let switched = f.ok(
        "todo_update",
        json!({"id": second, "status": "in_progress"}),
    );
    assert!(
        switched.contains(&format!(
            "(paused: {first} \"Write parser\" — back to pending)"
        )),
        "{switched}"
    );

    let done = f.ok("todo_update", json!({"id": second, "status": "done", "summary": "s", "verification": "v", "commitSubject": "c"}));
    assert!(
        done.contains(&format!("next: {first} \"Write parser\"")),
        "{done}"
    );
    let reopened = f.ok("todo_update", json!({"id": second, "status": "pending"}));
    assert!(reopened.starts_with("Updated: [ ] Test parser"));
    let plan = f.store.plan(&f.scope).unwrap();
    let item = thinkrail_plan_review::plan_ops::items(&plan)
        .find(|i| i.title == "Test parser")
        .unwrap();
    assert_eq!(
        (
            item.status,
            item.summary.as_deref(),
            item.commit_subject.as_deref()
        ),
        (PlanStatus::Pending, None, None)
    );

    assert!(
        f.ok("todo_list", json!({"status": "done"}))
            .contains("No done TODOs.")
    );
}

#[test]
fn plan_complete_nudge_and_summary() {
    let f = fixture();
    f.ok("todo_add", json!({"title": "Only step", "group": "Task"}));
    let id = f.id_of("Only step");
    let done = f.ok("todo_update", json!({"id": id, "status": "done"}));
    assert!(
        done.contains("plan complete — write a cumulative overall summary"),
        "{done}"
    );
    assert_eq!(
        f.ok("todo_plan_summary", json!({"summary": "## Summary\nx"})),
        "Plan summary saved."
    );
    assert_eq!(
        f.store.plan(&f.scope).unwrap().summary.as_deref(),
        Some("## Summary\nx")
    );
    assert_eq!(
        f.ok("todo_plan_summary", json!({"summary": "  "})),
        "Plan summary cleared."
    );
}

#[test]
fn loose_items_promote_and_cannot_anchor_inserts() {
    let f = fixture();
    f.ok("todo_add", json!({"title": "User idea"}));
    let loose = f.id_of("User idea");
    let anchored = f.call("todo_add", json!({"title": "x", "after": loose}));
    assert!(
        anchored.is_error && anchored.text.contains("is a loose item"),
        "{}",
        anchored.text
    );
    let promoted = f.ok("todo_promote", json!({"id": loose}));
    assert!(
        promoted.starts_with(&format!(
            "Promoted {loose} → group \"User idea\" ({loose})."
        )),
        "{promoted}"
    );
    let again = f.call("todo_promote", json!({"id": loose}));
    assert!(again.is_error);
    f.ok("todo_add", json!({"title": "Step", "group": "User idea"}));
    let step = f.id_of("Step");
    f.ok("todo_add", json!({"title": "After step", "after": step}));
    assert!(f.ok("todo_list", json!({})).contains("After step"));
}

#[test]
fn write_keeps_user_items_and_remove_reports_missing_ids() {
    let f = fixture();
    f.store
        .update_plan(&f.scope, &mut |plan| {
            thinkrail_plan_review::plan_ops::add(
                plan,
                thinkrail_plan_review::plan_ops::NewItem {
                    title: "Mine".into(),
                    origin: Some(thinkrail_core::Origin::User),
                    ..Default::default()
                },
            )
            .map(drop)
        })
        .unwrap();
    let wrote = f.ok("todo_write", json!({"groups": [{"title": "Task", "todos": [{"title": "A"}, {"title": "B", "status": "in_progress"}]}]}));
    assert!(
        wrote.starts_with("Wrote the plan (3 item(s) total):"),
        "{wrote}"
    );
    assert!(f.ok("todo_list", json!({})).contains("Mine"));
    let a = f.id_of("A");
    assert_eq!(
        f.ok("todo_remove", json!({"id": a})),
        format!("Removed {a}.")
    );
    let missing = f.call("todo_remove", json!({"id": a}));
    assert!(
        missing.is_error && missing.text == format!("No TODO with id \"{a}\"."),
        "{}",
        missing.text
    );
}

#[test]
fn bad_arguments_are_tool_errors_not_failures() {
    let f = fixture();
    for (name, args) in [
        ("todo_add", json!({})),
        ("todo_update", json!({"id": "x", "status": "finished"})),
        ("todo_list", json!({"offset": -1})),
        ("todo_write", json!({"groups": "nope"})),
        ("resolve_comment", json!({})),
        ("no_such_tool", json!({})),
    ] {
        assert!(f.call(name, args).is_error, "{name} should be a tool error");
    }
}

#[test]
fn resolve_comment_only_accepts_sent_comments_of_this_chat() {
    let f = fixture();
    let comment = f
        .store
        .add_review(
            &f.scope,
            NewReviewComment {
                path: "a.txt".into(),
                line: 1,
                text_quote: None,
                body: "rename this".into(),
            },
        )
        .unwrap();
    let draft = f.call(
        "resolve_comment",
        json!({"commentId": comment.id.to_string()}),
    );
    assert!(
        draft.is_error,
        "a draft comment cannot be resolved by the Agent"
    );
    f.store.send_package(&f.scope).unwrap();
    let other = PlanScope {
        workspace: "w".into(),
        chat: "other".into(),
    };
    assert!(
        f.tools
            .call(
                &other,
                HostToolOptions::default(),
                "resolve_comment",
                json!({"commentId": comment.id.to_string()})
            )
            .unwrap()
            .is_error
    );
    let resolved = f.ok(
        "resolve_comment",
        json!({"commentId": comment.id.to_string(), "note": "renamed"}),
    );
    assert_eq!(
        resolved,
        format!("Resolved review comment {} (\"rename this\").", comment.id)
    );
    assert_eq!(
        f.store.reviews(&f.scope).unwrap()[0].state,
        ReviewState::Resolved
    );
}

/// Records cancels; `slow` blocks until one arrives.
#[derive(Default)]
struct Fake {
    cancels: std::sync::Mutex<usize>,
    released: std::sync::Condvar,
}

impl mcp::Backend for Fake {
    fn list(&self) -> std::result::Result<Vec<HostToolSpec>, String> {
        Ok(vec![HostToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        }])
    }

    fn call(&self, name: &str, arguments: Value) -> std::result::Result<HostToolResult, String> {
        if name == "slow" {
            let mut cancels = self.cancels.lock().unwrap();
            while *cancels == 0 {
                cancels = self.released.wait(cancels).unwrap();
            }
        }
        Ok(HostToolResult {
            text: format!("{name}:{arguments}"),
            is_error: name == "bad",
            structured: None,
        })
    }

    fn cancel(&self) {
        *self.cancels.lock().unwrap() += 1;
        self.released.notify_all();
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serve_mcp(frames: &[Value], backend: Arc<Fake>) -> Vec<Value> {
    let input = frames
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let output = Shared::default();
    mcp::serve(&mut Cursor::new(input), output.clone(), backend).unwrap();
    let bytes = output.0.lock().unwrap().clone();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn mcp_negotiates_lists_calls_and_rejects_unknown_methods() {
    let frames = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"bad","arguments":{"x":1}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"resources/list"}),
        json!({"jsonrpc":"2.0","id":6,"method":"ping"}),
    ];
    let mut replies = serve_mcp(&frames, Arc::default());
    // A call is answered from its own thread, so it may land after later replies.
    replies.sort_by_key(|r| r["id"].as_i64());
    assert_eq!(replies.len(), 6, "the notification gets no reply");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(replies[1]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        replies[2]["result"]["tools"][0]["inputSchema"],
        json!({"type": "object"})
    );
    assert_eq!(replies[3]["result"]["content"][0]["text"], "bad:{\"x\":1}");
    assert_eq!(replies[3]["result"]["isError"], true);
    assert_eq!(replies[4]["error"]["code"], -32601);
    assert_eq!(replies[5]["result"], json!({}));
}

#[test]
fn a_cancelled_call_is_withdrawn_and_gets_no_reply() {
    let backend = Arc::new(Fake::default());
    let frames = [
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"slow","arguments":{}}}),
        // Not running: ignored, no cancel reaches the host.
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":99}}),
        json!({"jsonrpc":"2.0","id":8,"method":"ping"}),
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}),
    ];
    let replies = serve_mcp(&frames, backend.clone());
    // The loop kept reading while `slow` blocked; its reply is withheld.
    assert_eq!(replies, [json!({"jsonrpc":"2.0","id":8,"result":{}})]);
    assert_eq!(*backend.cancels.lock().unwrap(), 1);
}

#[test]
fn pipe_bridge_scopes_calls_by_token_and_rejects_strangers() {
    let f = fixture();
    let handler: Arc<dyn HostToolHandler> = Arc::new(f.tools.clone());
    let bridge = ToolBridge::start(handler).unwrap();
    let token = bridge.register(f.scope.clone(), HostToolOptions::default());

    let mut client = BridgeClient::connect(bridge.pipe_name(), &token).unwrap();
    let listed = client.request(json!({"method": "list"})).unwrap();
    assert_eq!(listed["result"].as_array().map(Vec::len), Some(15));
    let added = client
        .request(json!({"method": "call", "name": "todo_add", "arguments": {"title": "Via pipe"}}))
        .unwrap();
    assert_eq!(added["result"]["isError"], false, "{added}");
    assert!(
        f.ok("todo_list", json!({})).contains("Via pipe"),
        "the call landed in the token's scope"
    );

    let mut stranger = BridgeClient::connect(bridge.pipe_name(), "not-a-token").unwrap();
    assert!(
        stranger.request(json!({"method": "list"})).is_err(),
        "unknown tokens are disconnected"
    );

    bridge.revoke(&token);
    assert!(
        client.request(json!({"method": "list"})).is_err(),
        "revoked tokens end the connection"
    );
}
