use super::*;

#[test]
fn default_routing_is_empty() {
    assert!(EngineRouting::default().is_empty());
}

#[test]
fn routing_with_env_is_not_empty_and_hides_values() {
    let routing = EngineRouting { env: vec![("THINKRAIL_X".into(), Secret::new("sk-test-a"))], ..EngineRouting::default() };
    assert!(!routing.is_empty());
    assert!(!format!("{routing:?}").contains("sk-test-a"));

    let target = ProbeTarget {
        endpoint: Endpoint { protocol: Protocol::OpenaiChat, base_url: "http://127.0.0.1:1".into() },
        headers: vec![Header { name: "X-Token".into(), value: "hdr-secret".into() }],
        key: Some(Secret::new("sk-test-a")),
    };
    let text = format!("{target:?}");
    assert!(!text.contains("sk-test-a") && !text.contains("hdr-secret"), "{text}");
}
