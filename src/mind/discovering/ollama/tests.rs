use super::*;

/// A model as `/v1/models` gives it: a name and nothing else, so everything is assumed.
fn listed(id: &str) -> Model {
    crate::mind::discovering::parse(&serde_json::json!({
        "data": [{ "id": id, "object": "model", "owned_by": "library" }]
    }))
    .remove(0)
}

/// `/api/show` for a small reasoning model, trimmed to what is read.
fn shown(capabilities: &[&str], window: u64) -> serde_json::Value {
    serde_json::json!({
        "capabilities": capabilities,
        "model_info": { "qwen3.context_length": window, "qwen3.embedding_length": 1024 },
    })
}

#[test]
fn the_window_is_ollamas_own_rather_than_the_one_assumed() {
    // A 41k model assumed to hold 128k builds a request the daemon refuses mid-turn.
    let model = apply(listed("qwen3:0.6b"), &shown(&["completion"], 40_960));
    assert_eq!(model.context_window, 40_960);
    assert_eq!(model.max_tokens, 10_240, "a quarter of it");
}

#[test]
fn a_model_that_thinks_is_marked_as_one() {
    let model = apply(
        listed("qwen3:0.6b"),
        &shown(&["completion", "thinking"], 40_960),
    );
    assert!(
        model.reasoning,
        "or no level is ever sent and nothing is shown"
    );
}

#[test]
fn off_is_said_in_ollamas_word() {
    // "off" is refused with an error; "none" is what it means.
    let model = apply(listed("qwen3:0.6b"), &shown(&["thinking"], 40_960));
    assert_eq!(
        model.thinking.get(&ThinkingLevel::Off),
        Some(&Some("none".to_owned()))
    );
}

#[test]
fn a_model_that_does_not_think_is_left_as_one_that_does_not() {
    let model = apply(listed("llama3:8b"), &shown(&["completion", "tools"], 8_192));
    assert!(!model.reasoning);
    assert!(model.thinking.is_empty());
}

#[test]
fn a_model_that_sees_takes_images() {
    let model = apply(
        listed("qwen3.5:122b"),
        &shown(&["completion", "vision"], 262_144),
    );
    assert!(model.input.contains(&Modality::Image));
    let again = apply(model, &shown(&["vision"], 262_144));
    assert_eq!(
        again
            .input
            .iter()
            .filter(|m| **m == Modality::Image)
            .count(),
        1,
        "not twice"
    );
}

#[test]
fn an_answer_with_no_window_keeps_the_assumed_one() {
    let before = listed("qwen3:0.6b");
    let window = before.context_window;
    let model = apply(
        before,
        &serde_json::json!({ "capabilities": ["completion"] }),
    );
    assert_eq!(model.context_window, window);
}

#[test]
fn the_root_is_found_beside_the_openai_base() {
    assert_eq!(root("http://10.0.0.1:11434/v1"), "http://10.0.0.1:11434");
    assert_eq!(root("http://10.0.0.1:11434/v1/"), "http://10.0.0.1:11434");
    assert_eq!(root("http://10.0.0.1:11434"), "http://10.0.0.1:11434");
}
