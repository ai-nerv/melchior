use super::*;
use crate::mind::provider::api::Delta;
use crate::mind::provider::testing::Step;

fn rejection(output: u64, room: u64) -> Vec<Step> {
    reply(400, json!({"error": {"type": "BadRequestError", "code": 400,
        "message": format!("'max_tokens' or 'max_completion_tokens' is too large: {output}. This model's maximum context length is {} tokens and your request has 12 input tokens ({output} > {} - 12).", room + 12, room + 12)
    }}).to_string().as_bytes())
}

fn success() -> Vec<Step> {
    reply(200, b"data: {\"choices\":[{\"delta\":{\"content\":\"recovered\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
}

struct Captured {
    bodies: Vec<serde_json::Value>,
    retries: Vec<super::super::Retrying>,
    deltas: Vec<Delta>,
    result: Result<(), super::super::ProviderError>,
}

async fn run(replies: Vec<Vec<Step>>, options: Options, lua: &str) -> Captured {
    let server = Server::start(replies).await;
    let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
        "base_url":server.url,"api":"openai-completions","auth":{"kind":"none"},
        "models":[{"id":"fixture","name":"Fixture","context_window":8000,"max_tokens":900,
            "reasoning":true}]}))
    .expect("provider");
    let mut engine = engine_with_builtins().expect("builtins");
    engine.run(lua, "budget fixture").expect("fixture");
    let adapter = LuaAdapter::new(engine, "openai-completions").expect("adapter");
    let context = Context {
        system: Some("exact rules\r\n".into()),
        messages: vec![Message::user("exact input")],
        ..Default::default()
    };
    let original = context.clone();
    let (mut retries, mut deltas) = (Vec::new(), Vec::new());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        Client::with_base_delay(Duration::ZERO).stream_reporting(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &provider.models[0],
                context: &context,
                options: &options,
            },
            |delta| deltas.push(delta),
            |retry| retries.push(retry),
        ),
    )
    .await
    .expect("bounded retry");
    assert_eq!(context, original);
    let bodies = server
        .bodies
        .lock()
        .expect("bodies")
        .iter()
        .map(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).expect("JSON"))
        .collect::<Vec<_>>();
    assert_eq!(server.requests.load(Ordering::SeqCst), bodies.len());
    assert!(
        bodies
            .iter()
            .all(|body| body.get("__budget").is_none() && body.get("__scratch").is_none())
    );
    Captured {
        bodies,
        retries,
        deltas,
        result,
    }
}

#[tokio::test]
async fn automatic_output_recovers_without_changing_any_other_request_bytes() {
    let seen = run(vec![rejection(900, 52), success()], Options::default(), "").await;
    seen.result.expect("recovered");
    assert_eq!(seen.bodies.len(), 2);
    assert_eq!(seen.retries.len(), 1);
    assert_eq!(seen.bodies[0]["max_tokens"], 900);
    assert_eq!(seen.bodies[1]["max_tokens"], 52);
    let mut next = seen.bodies[1].clone();
    next["max_tokens"] = json!(900);
    assert_eq!(seen.bodies[0], next);
    assert!(seen.deltas.contains(&Delta::Text("recovered".into())));
}

#[tokio::test]
async fn explicit_requests_and_session_limits_never_reduce() {
    for (options, lua) in [
        (
            Options {
                max_tokens: Some(77),
                ..Default::default()
            },
            "",
        ),
        (Options::default(), "melchior.max_tokens = 77"),
        (
            Options {
                max_tokens: Some(900),
                max_tokens_automatic: true,
                ..Default::default()
            },
            "melchior.max_tokens = 77",
        ),
    ] {
        let seen = run(vec![rejection(77, 52)], options, lua).await;
        assert!(seen.result.is_err());
        assert_eq!(seen.bodies.len(), 1);
        assert_eq!(seen.bodies[0]["max_tokens"], 77);
        assert!(seen.retries.is_empty());
    }
}

#[tokio::test]
async fn ordinary_and_output_retries_share_four_total_attempts() {
    let seen = run(
        vec![
            reply(500, b"busy"),
            rejection(900, 600),
            reply(500, b"busy"),
            rejection(600, 300),
        ],
        Options::default(),
        "",
    )
    .await;
    assert!(seen.result.is_err());
    assert_eq!(
        seen.bodies
            .iter()
            .map(|body| body["max_tokens"].as_u64().expect("output limit"))
            .collect::<Vec<_>>(),
        [900, 900, 600, 600]
    );
    assert_eq!(
        seen.retries
            .iter()
            .map(|retry| (retry.attempt, retry.max_attempts))
            .collect::<Vec<_>>(),
        [(1, 4), (2, 4), (3, 4)]
    );
}

#[tokio::test]
async fn repeated_output_rejections_are_strictly_decreasing_and_bounded() {
    let seen = run(
        vec![
            rejection(900, 600),
            rejection(600, 300),
            rejection(300, 150),
            rejection(150, 75),
        ],
        Options::default(),
        "",
    )
    .await;
    assert!(seen.result.is_err());
    assert_eq!(
        seen.bodies
            .iter()
            .map(|body| body["max_tokens"].as_u64().expect("output limit"))
            .collect::<Vec<_>>(),
        [900, 600, 300, 150]
    );
    assert_eq!(seen.retries.len(), 3);
}

#[tokio::test]
async fn missing_invalid_or_inconsistent_budget_metadata_stays_terminal() {
    for change in [
        "body.__budget = nil",
        "body.__budget.version = 2",
        "body.__budget.effective = 901",
        "body.__budget.minimum = 0",
        "body.__budget.minimum = 901",
        "body.__budget.path = '/missing'",
        "body.__budget.automatic = false",
        "body.__budget.extra = true",
    ] {
        let lua = format!(
            "local api = melchior.apis['openai-completions']; local original = api.request; api.request = function(...) local body = original(...); {change}; return body end"
        );
        let seen = run(vec![rejection(900, 52)], Options::default(), &lua).await;
        assert!(seen.result.is_err(), "{change}");
        assert_eq!(seen.bodies.len(), 1, "{change}");
        assert!(seen.retries.is_empty(), "{change}");
    }
}

#[tokio::test]
async fn changed_payload_or_thinking_and_mismatched_reported_counts_never_retry() {
    for (options, lua, count) in [
        (Options::default(), "", 899),
        (
            Options {
                thinking: Some(crate::mind::model::ThinkingLevel::High),
                ..Default::default()
            },
            "",
            900,
        ),
        (
            Options::default(),
            "local api = melchior.apis['openai-completions']; local original = api.request; api.request = function(model, ctx, opts) local body = original(model, ctx, opts); if opts.__output_limit then body.messages = {} end; return body end",
            900,
        ),
        (
            Options::default(),
            "local api = melchior.apis['openai-completions']; local original = api.request; api.request = function(model, ctx, opts) local body = original(model, ctx, opts); body.__scratch = { amount = opts.__output_limit or 900 }; return body end",
            900,
        ),
    ] {
        let seen = run(vec![rejection(count, 52)], options, lua).await;
        assert!(seen.result.is_err());
        assert_eq!(seen.bodies.len(), 1);
        assert!(seen.retries.is_empty());
    }
}

#[tokio::test]
async fn cancellation_before_and_during_output_adjustment_closes_without_more_attempts() {
    use crate::mind::provider::testing::headers;
    for adjusted in [false, true] {
        let mut replies = Vec::new();
        if adjusted {
            replies.push(rejection(900, 52));
        }
        replies.push(vec![headers(400), Step::Hold]);
        replies.push(success());
        let server = Server::start(replies).await;
        let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
            "base_url":server.url,"api":"openai-completions","auth":{"kind":"none"},
            "models":[{"id":"fixture","name":"Fixture","context_window":8000,"max_tokens":900}]}))
        .expect("provider");
        let adapter = LuaAdapter::new(
            engine_with_builtins().expect("builtins"),
            "openai-completions",
        )
        .expect("adapter");
        let context = Context::default();
        let options = Options::default();
        let client = Client::with_base_delay(Duration::ZERO);
        let call = Call {
            adapter: &adapter,
            provider: &provider,
            model: &provider.models[0],
            context: &context,
            options: &options,
        };
        let attempts = 1 + usize::from(adjusted);
        let mut retries = 0;
        {
            let waiting = client.stream_reporting(&call, |_| {}, |_| retries += 1);
            tokio::pin!(waiting);
            tokio::select! {
                result = &mut waiting => panic!("held rejection ended: {result:?}"),
                () = server.wait_for(&server.requests, attempts) => {}
            }
        }
        server.wait_for(&server.closed, 1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(server.requests.load(Ordering::SeqCst), attempts);
        assert_eq!(retries, usize::from(adjusted));
        let bodies = server.bodies.lock().expect("bodies");
        let last: serde_json::Value =
            serde_json::from_slice(bodies.last().expect("request")).expect("JSON");
        assert_eq!(last["max_tokens"], if adjusted { 52 } else { 900 });
    }
}
