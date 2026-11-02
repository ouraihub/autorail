use serde_json::{Value, json};
use thinkrail_core::{
    ContentBlock, ErrorKind, Interaction, InteractionAnswer, InteractionKind, Role, SessionEvent,
    ThinkingLevel, ToolStatus,
};

use super::{
    dialogs::{UiRequest, cancel, ui_request, ui_response},
    model::switch_commands,
    *,
};
use crate::projection::tests::projection;

fn shown(frame: Value) -> Interaction {
    match ui_request(&frame).unwrap() {
        UiRequest::Show(interaction, _) => interaction,
        _ => panic!("expected a question"),
    }
}

#[test]
fn pi_stream_without_message_ids_builds_one_assistant_message() {
    let (mut p, events) = projection();
    let mut stream = Stream::default();
    stream.message_start(
        &mut p,
        &json!({"type":"message_start","message":{"role":"user","content":"hi","timestamp":1}}),
    );
    stream.message_end(
        &mut p,
        &json!({"type":"message_end","message":{"role":"user","content":"hi","timestamp":1}}),
    );
    let start = json!({"type":"message_start","message":{"role":"assistant","content":[],"stopReason":"pending"}});
    stream.message_start(&mut p, &start);
    let update = |event: Value| json!({"type":"message_update","assistantMessageEvent":event});
    stream.message_update(
        &mut p,
        &update(json!({"type":"thinking_start","contentIndex":0})),
    );
    stream.message_update(
        &mut p,
        &update(json!({"type":"thinking_delta","contentIndex":0,"delta":"plan"})),
    );
    stream.message_update(
        &mut p,
        &update(json!({"type":"text_start","contentIndex":1})),
    );
    stream.message_update(
        &mut p,
        &update(json!({"type":"text_delta","contentIndex":1,"delta":"Hel"})),
    );
    stream.message_update(
        &mut p,
        &update(json!({"type":"text_delta","contentIndex":1,"delta":"lo"})),
    );
    stream.message_update(
        &mut p,
        &update(json!({"type":"toolcall_start","contentIndex":2,"id":"t1","toolName":"read"})),
    );
    stream.message_update(
        &mut p,
        &update(
            json!({"type":"toolcall_end","contentIndex":2,"toolCall":{"arguments":{"path":"a"}}}),
        ),
    );
    let end = json!({"type":"message_end","message":{"role":"assistant","stopReason":"toolUse","content":[
        {"type":"thinking","thinking":"plan"},{"type":"text","text":"Hello"},
        {"type":"toolCall","id":"t1","name":"read","arguments":{"path":"a"}}]}});
    stream.message_end(&mut p, &end);
    tool_event(
        &mut p,
        &json!({"type":"tool_execution_end","toolCallId":"t1","isError":false,
        "result":{"content":[{"type":"text","text":"body"}]}}),
    );

    let snapshot = p.snapshot();
    assert_eq!(
        snapshot.messages.len(),
        2,
        "one user and one assistant message"
    );
    assert_eq!(snapshot.messages[0].role, Role::User);
    let assistant = &snapshot.messages[1];
    assert_eq!(
        assistant.content[0],
        ContentBlock::Thinking {
            text: "plan".into()
        }
    );
    assert_eq!(
        assistant.content[1],
        ContentBlock::Text {
            text: "Hello".into()
        }
    );
    assert!(matches!(&assistant.content[2],
        ContentBlock::Tool { status: ToolStatus::Completed, output: Some(o), input: Some(i), .. } if o == "body" && i["path"] == "a"));
    let deltas: Vec<(usize, String)> = events
        .0
        .lock()
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Delta {
                block,
                text,
                message_id,
                ..
            } if *message_id == assistant.id => Some((*block, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec![(0, "plan".into()), (1, "Hel".into()), (1, "lo".into())]
    );
}

#[test]
fn omp_delta_frames_use_the_engine_message_id() {
    let (mut p, _) = projection();
    let mut stream = Stream::default();
    stream.message_start(&mut p, &json!({"type":"message_start","messageId":"m7","message":{"role":"assistant","content":[]}}));
    let update = json!({"type":"message_update","messageId":"m7","message":{"role":"assistant"},
        "assistantMessageEvent":{"type":"text_start","contentIndex":0,"block":{"type":"text","text":""}}});
    stream.message_update(&mut p, &update);
    stream.message_update(
        &mut p,
        &json!({"type":"message_update","messageId":"m7",
        "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"ok"}}),
    );
    assert_eq!(p.snapshot().messages[0].id, "m7");
    assert_eq!(
        p.snapshot().messages[0].content,
        vec![ContentBlock::Text { text: "ok".into() }]
    );
}

#[test]
fn assistant_failures_are_shown_and_hidden_custom_messages_are_not() {
    let (mut p, _) = projection();
    let mut stream = Stream::default();
    stream.message_end(
        &mut p,
        &json!({"type":"message_end","message":{"role":"assistant","content":[],
        "stopReason":"error","errorMessage":"401 bad api_key=sk-123"}}),
    );
    replay(
        &mut p,
        &[json!({"role":"custom","display":false,"content":"internal"})],
        0,
    );
    let snapshot = p.snapshot();
    assert_eq!(
        snapshot.messages.len(),
        2,
        "the failed message plus its visible note"
    );
    let ContentBlock::Text { text } = &snapshot.messages[1].content[0] else {
        panic!("note")
    };
    assert!(text.contains("401") && !text.contains("sk-123"), "{text}");
    assert_eq!(snapshot.last_error.map(|e| e.code), Some("EXTERNAL".into()));
}

#[test]
fn replayed_tool_results_attach_to_their_calls() {
    let (mut p, _) = projection();
    replay(
        &mut p,
        &[
            json!({"role":"assistant","content":[{"type":"toolCall","id":"t","name":"bash","arguments":{}}]}),
            json!({"role":"toolResult","toolCallId":"t","isError":true,"content":[{"type":"text","text":"boom"}]}),
        ],
        0,
    );
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), 1);
    assert!(
        matches!(&snapshot.messages[0].content[0], ContentBlock::Tool { status: ToolStatus::Failed, output: Some(o), .. } if o == "boom")
    );
}

#[test]
fn dialog_requests_map_to_interactions_and_answers_to_engine_replies() {
    let select = shown(
        json!({"type":"extension_ui_request","id":"s","method":"select","title":"Run?","options":["Allow","Deny"],"timeout":5000}),
    );
    assert_eq!(select.kind, InteractionKind::Select);
    assert_eq!(select.options[1].label, "Deny");
    let reply = ui_response(
        &select,
        &InteractionAnswer::Select {
            option_id: "1".into(),
        },
    )
    .unwrap();
    assert_eq!(
        reply,
        json!({"type":"extension_ui_response","id":"s","value":"Deny"})
    );
    assert_eq!(
        ui_response(
            &select,
            &InteractionAnswer::Select {
                option_id: "9".into()
            }
        )
        .unwrap_err()
        .kind(),
        Some(ErrorKind::Invalid)
    );

    let confirm = shown(
        json!({"type":"extension_ui_request","id":"c","method":"confirm","title":"t","message":"m"}),
    );
    assert_eq!(
        ui_response(&confirm, &InteractionAnswer::Confirm { confirmed: false }).unwrap(),
        json!({"type":"extension_ui_response","id":"c","confirmed":false})
    );
    assert!(
        ui_response(&confirm, &InteractionAnswer::Text { text: "x".into() }).is_err(),
        "answer must fit the question"
    );
    assert_eq!(
        ui_response(&confirm, &InteractionAnswer::Cancel).unwrap(),
        cancel("c")
    );

    let ask = shown(
        json!({"type":"extension_ui_request","id":"a","method":"ask","questions":[{"id":"q1","question":"?","options":[]}]}),
    );
    let answers = json!([{"id":"q1","selectedOptions":[],"customInput":"free"}]);
    assert_eq!(
        ui_response(
            &ask,
            &InteractionAnswer::Answers {
                answers: answers.clone()
            }
        )
        .unwrap(),
        json!({"type":"extension_ui_response","id":"a","answers":answers})
    );
    assert!(
        ui_response(&ask, &InteractionAnswer::Answers { answers: json!([]) }).is_err(),
        "every question needs an answer"
    );

    assert!(matches!(
        ui_request(&json!({"method":"setWidget","id":"w"})).unwrap(),
        UiRequest::Ignore
    ));
    assert!(
        matches!(ui_request(&json!({"method":"cancel","id":"x","targetId":"s"})).unwrap(), UiRequest::Withdraw(t) if t == "s")
    );
    assert!(
        ui_request(&json!({"method":"select","id":"s"})).is_err(),
        "select without options is a protocol error"
    );
}

#[test]
fn model_switch_sends_set_model_then_the_level() {
    let frames: Vec<Value> = switch_commands(
        "thinkrail-1a2b3c4d-am",
        "claude-x",
        Some(ThinkingLevel::Xhigh),
    )
    .into_iter()
    .map(|(kind, mut fields)| {
        fields["type"] = json!(kind);
        fields
    })
    .collect();
    assert_eq!(
        frames,
        [
            json!({"type":"set_model","provider":"thinkrail-1a2b3c4d-am","modelId":"claude-x"}),
            json!({"type":"set_thinking_level","level":"xhigh"}),
        ]
    );
    let kinds: Vec<&str> = switch_commands("p", "m", None)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(
        kinds,
        ["set_model"],
        "no level keeps the engine's own clamp"
    );
}

#[test]
fn state_effort_is_read_from_thinking_level() {
    let state = json!({"model":{"provider":"p","id":"m"},"thinkingLevel":"high"});
    let info = model(&state).unwrap();
    assert_eq!(
        (info.provider.as_deref(), info.id.as_str(), info.effort),
        (Some("p"), "m", Some(ThinkingLevel::High))
    );
    for unknown in [
        json!({"model":{"id":"m"},"thinkingLevel":"inherit"}),
        json!({"model":{"id":"m"}}),
    ] {
        assert_eq!(model(&unknown).unwrap().effort, None);
    }
    for level in crate::projection_engine::LEVELS {
        assert_eq!(
            thinking_level(crate::projection_engine::level_name(level)),
            Some(level)
        );
    }
    assert_eq!(thinking_level(""), None);
}

#[test]
fn engine_level_and_config_changes_publish_once() {
    let (mut p, events) = projection();
    p.set_model(model(
        &json!({"model":{"provider":"p","id":"m"},"thinkingLevel":"low"}),
    ));
    let published = || {
        events
            .0
            .lock()
            .iter()
            .filter(|e| matches!(e, SessionEvent::Snapshot { .. }))
            .count()
    };
    let before = published();
    model_event(
        &mut p,
        &json!({"type":"thinking_level_changed","level":"high"}),
    );
    assert_eq!(p.model().unwrap().effort, Some(ThinkingLevel::High));
    model_event(
        &mut p,
        &json!({"type":"thinking_level_changed","thinkingLevel":"high"}),
    );
    assert_eq!(
        published(),
        before + 1,
        "an unchanged level publishes nothing"
    );
    model_event(
        &mut p,
        &json!({"type":"config_update","model":{"provider":"q","id":"n"},"thinkingLevel":"off"}),
    );
    let info = p.model().unwrap();
    assert_eq!(
        (info.provider.as_deref(), info.id.as_str(), info.effort),
        (Some("q"), "n", Some(ThinkingLevel::Off))
    );
    assert_eq!(published(), before + 2);
}
