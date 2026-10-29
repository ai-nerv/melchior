use super::*;

#[test]
fn a_stream_that_finishes_with_an_error_is_asked_again() {
    let why = completed(Some("error"), "openrouter").expect_err("an error is no answer");
    assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
    for real in [Some("stop"), Some("length"), Some("tool_calls"), None] {
        assert!(completed(real, "openrouter").is_ok(), "{real:?}");
    }
}

#[test]
fn a_stream_that_says_nothing_at_all_is_asked_again() {
    let why = hung("openrouter");
    assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
    assert!(why.to_string().contains("180"), "{why}");
}

#[test]
fn a_tool_call_written_as_text_is_asked_again() {
    let leaked = "Let me look.\n\n<｜DSML｜tool_cinvoke name=\"shell\">";
    let why = unleaked(leaked, "openrouter").expect_err("a leaked call is not an answer");
    assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
    assert!(unleaked("I wrote the doc.", "openrouter").is_ok());
}

#[test]
fn the_finish_a_provider_named_is_found_in_any_spelling() {
    let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"cost":0.1}}"#;
    assert_eq!(reason_in(chunk).as_deref(), Some("stop"));
    assert_eq!(
        reason_in(r#"{"delta":{"stop_reason":"end_turn"}}"#).as_deref(),
        Some("end_turn")
    );
    assert_eq!(
        reason_in(r#"{"finishReason":"STOP"}"#).as_deref(),
        Some("STOP")
    );
    assert_eq!(reason_in("[DONE]"), None);
    let routed = r#"{"id":"gen-1","provider":"StreamLake","model":"deepseek/deepseek-v4-flash"}"#;
    assert_eq!(value_in(routed, "provider").as_deref(), Some("StreamLake"));
    let mut tail = Vec::new();
    for data in ["first", "second", "third"] {
        kept(&mut tail, data);
    }
    assert_eq!(tail, ["second", "third"]);
}

#[test]
fn a_stream_that_never_stopped_is_retried_not_kept() {
    assert!(finished(true, "p").is_ok());
    let cut = finished(false, "p").expect_err("a cut-off stream");
    assert!(cut.class.is_retryable(), "{}", cut.message);
}

#[test]
fn an_upstream_failing_inside_the_stream_is_said_as_it_was_said() {
    let failed = r#"{"id":"gen-1","model":"unknown","provider":"Novita","choices":[],"error":{"code":504,"message":"The operation was aborted","metadata":{"error_type":"timeout"}}}"#;
    let why = unfailed(&[failed.to_owned()], "openrouter").expect_err("a failed stream");
    assert_eq!(why.class, RetryClass::Overload);
    for part in ["Novita", "The operation was aborted", "504"] {
        assert!(why.message.contains(part), "{part}: {}", why.message);
    }
    let fine = r#"{"choices":[{"delta":{"content":"an error occurred to me"}}]}"#;
    assert!(unfailed(&[fine.to_owned()], "openrouter").is_ok());
}

#[test]
fn an_overflow_said_inside_the_stream_is_an_overflow() {
    let long = r#"{"provider":"DeepInfra","choices":[],"error":{"code":400,"message":"Upstream error from DeepInfra: Requested input length 35327 exceeds maximum input length 32767"}}"#;
    let why = unfailed(&[long.to_owned()], "openrouter").expect_err("too long");
    assert_eq!(why.class, RetryClass::Overflow, "{}", why.message);
    // Anything else said mid-stream is still asked again: the request was taken, so it was
    // not malformed, whatever status the upstream's own failure carried.
    let odd = r#"{"choices":[],"error":{"code":400,"message":"upstream hiccup"}}"#;
    let why = unfailed(&[odd.to_owned()], "openrouter").expect_err("failed");
    assert_eq!(why.class, RetryClass::Transport);
}
