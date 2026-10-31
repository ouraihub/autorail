use serde_json::json;
use thinkrail_core::{ContentBlock, ErrorKind, Interaction, InteractionAnswer, InteractionKind, InteractionOption, Role, ToolStatus};

use super::*;
use crate::projection::tests::projection;

#[test]
fn committed_chunks_of_one_message_append_blocks_in_order() {
    let (mut p, _) = projection();
    let chunk = |kind: &str, text: &str| {
        json!({"sessionUpdate": kind, "messageId": "m", "content": {"type": "text", "text": text}})
    };
    apply_update(&mut p, &chunk("agent_thought_chunk", "think"));
    apply_update(&mut p, &chunk("agent_message_chunk", "answer"));
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].role, Role::Assistant);
    assert_eq!(
        snapshot.messages[0].content,
        vec![ContentBlock::Thinking { text: "think".into() }, ContentBlock::Text { text: "answer".into() }]
    );
}

#[test]
fn tool_lifecycle_and_model_option() {
    let (mut p, _) = projection();
    apply_update(&mut p, &json!({"sessionUpdate":"tool_call","toolCallId":"c1","title":"read","status":"in_progress","rawInput":{"path":"a"}}));
    apply_update(&mut p, &json!({"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"failed",
        "content":[{"type":"content","content":{"type":"text","text":"denied"}}]}));
    apply_update(&mut p, &json!({"sessionUpdate":"config_option_update","configOptions":[
        {"id":"model","category":"model","currentValue":"[\"deepseek\",\"deepseek-v4\"]"}]}));
    let snapshot = p.snapshot();
    assert!(matches!(&snapshot.messages[0].content[0],
        ContentBlock::Tool { status: ToolStatus::Failed, output: Some(o), .. } if o == "denied"));
    let model = snapshot.model.unwrap();
    assert_eq!((model.provider.as_deref(), model.id.as_str()), (Some("deepseek"), "deepseek-v4"));
}

#[test]
fn permission_answers_must_name_an_offered_option() {
    let interaction = Interaction {
        request_id: "r".into(),
        kind: InteractionKind::Permission,
        title: String::new(),
        message: String::new(),
        options: vec![InteractionOption { id: "allow-once".into(), label: "Allow once".into() }],
        detail: Value::Null,
    };
    let allow = InteractionAnswer::Select { option_id: "allow-once".into() };
    assert_eq!(permission_outcome(&interaction, &allow).unwrap(), json!({"outcome":"selected","optionId":"allow-once"}));
    assert_eq!(permission_outcome(&interaction, &InteractionAnswer::Cancel).unwrap(), json!({"outcome":"cancelled"}));
    let forged = InteractionAnswer::Select { option_id: "allow-always".into() };
    assert_eq!(permission_outcome(&interaction, &forged).unwrap_err().kind(), Some(ErrorKind::Invalid));
    let wrong = InteractionAnswer::Confirm { confirmed: true };
    assert_eq!(permission_outcome(&interaction, &wrong).unwrap_err().kind(), Some(ErrorKind::Invalid));
}

#[test]
fn a_failed_turn_is_visible_in_the_transcript() {
    let (mut p, _) = projection();
    turn_failed(&mut p, "user-1-error".into(), &engine_error("turn failed: 401 bad api_key=sk-123"));
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].role, Role::System);
    let ContentBlock::Text { text } = &snapshot.messages[0].content[0] else { panic!("text note") };
    assert!(text.contains("401") && !text.contains("sk-123"), "{text}");
    assert_eq!(snapshot.last_error.map(|e| e.code), Some("EXTERNAL".into()));
}
