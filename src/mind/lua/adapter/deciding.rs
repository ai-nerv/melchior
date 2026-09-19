//! The `decisions` protocol: a schema becomes typed questions, and one JSON document of answers
//! becomes an answer of the schema's shape.

use super::support::*;
use super::*;
use crate::mind::model::StopReason;
use crate::mind::provider::api::{Schema, StreamState};
use crate::mind::provider::sse;

fn verdict() -> Options {
    Options {
        schema: Some(Schema {
            name: "verdict".into(),
            schema: serde_json::json!({ "type": "object", "properties": {
                "safe": { "type": "boolean", "description": "May it run without asking?",
                          "x-threshold": 0.8,
                          "x-criteria": { "true": "reads or builds", "false": "anything else" } },
                "rule": { "type": "string", "enum": ["read-only", "download-execute"],
                          "x-criteria": { "read-only": "only reads or lists",
                                          "download-execute": "downloads code and runs it" } },
                "heat": { "type": "number", "x-criteria": ["calm", "cross", "furious"] },
                "reason": { "type": "string", "x-from": "rule" },
            }}),
        }),
        ..Options::default()
    }
}

/// What the dialect says of `reply`, read by what it kept from the request for `options`.
fn answered(options: &Options, reply: &serde_json::Value) -> Vec<Delta> {
    let adapter = adapter("decisions");
    let mut body = adapter.request(&plain_model(), &plain_context(), options);
    let mut state = StreamState {
        scratch: body["__scratch"].take(),
        ..StreamState::default()
    };
    adapter.on_event(
        &mut state,
        &sse::Event {
            name: "body".into(),
            data: reply.to_string(),
        },
    )
}

fn said(deltas: &[Delta]) -> String {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_schema_is_asked_as_one_typed_question_to_a_property() {
    let body = adapter("decisions").request(&plain_model(), &plain_context(), &verdict());
    assert_eq!(body["model"], "m-1");
    assert_eq!(body["state"], "hi", "one message is the state as it stands");
    let asked = &body["questions"];
    assert_eq!(asked["safe"]["type"], "noul");
    assert_eq!(asked["safe"]["instructions"], "May it run without asking?");
    assert_eq!(asked["safe"]["criteria"]["false"], "anything else");
    assert_eq!(asked["rule"]["type"], "choice");
    assert_eq!(
        asked["rule"]["criteria"]["download-execute"],
        "downloads code and runs it"
    );
    assert_eq!(asked["heat"]["type"], "score");
    assert_eq!(
        asked["heat"]["criteria"],
        serde_json::json!(["calm", "cross", "furious"])
    );
    assert!(
        asked["reason"].is_null(),
        "such a model writes nothing, so it is not asked to"
    );
}

#[test]
fn the_door_is_the_routers_or_the_makers_own() {
    let at = |base: &str| adapter("decisions").endpoint(base, &plain_model());
    assert_eq!(
        at("https://openrouter.ai/api/alpha"),
        "https://openrouter.ai/api/alpha/decisions"
    );
    assert_eq!(
        at("https://api.typesafe.ai/v1/systemone"),
        "https://api.typesafe.ai/v1/systemone"
    );
}

#[test]
fn the_answers_come_back_in_the_schemas_shape_with_how_sure_beside_them() {
    let reply = serde_json::json!({
        "answers": {
            "safe": { "type": "noul", "noul": 0.74 },
            "rule": { "type": "choice", "choice": "download-execute",
                      "probabilities": { "read-only": 0.04, "download-execute": 0.96 },
                      "confidence": 0.95 },
            "heat": { "type": "score", "score": 1.6, "confidence": 0.7 },
        },
        "usage": { "input_tokens": 583, "output_tokens": 99, "cost": 0.000_024_4 },
    });
    let deltas = answered(&verdict(), &reply);
    let answer: serde_json::Value = serde_json::from_str(&said(&deltas)).expect("json");
    assert_eq!(
        answer["safe"], false,
        "0.74 is under the 0.8 this property asked for"
    );
    assert_eq!(answer["rule"], "download-execute");
    assert_eq!(answer["heat"], 1.6);
    assert_eq!(
        answer["reason"], "downloads code and runs it",
        "said for it, from its choice"
    );
    assert_eq!(answer["_decided"]["safe"]["p"], 0.74);
    assert_eq!(answer["_decided"]["rule"]["confidence"], 0.95);
    assert!(deltas.contains(&Delta::Stop(StopReason::EndTurn)));
    let usage = deltas.iter().find_map(|delta| match delta {
        Delta::Usage(usage) => Some(*usage),
        _ => None,
    });
    let usage = usage.expect("usage");
    assert_eq!(
        (usage.input, usage.output, usage.cost_micros),
        (583, 99, 24)
    );
}

#[test]
fn asked_with_no_shape_it_says_yes_or_no() {
    let plain = Options::default();
    let body = adapter("decisions").request(&plain_model(), &plain_context(), &plain);
    assert_eq!(body["questions"]["answer"]["type"], "noul");
    let reply = serde_json::json!({ "answers": { "answer": { "type": "noul", "noul": 0.91 } } });
    assert_eq!(said(&answered(&plain, &reply)), "yes (0.91)");
}

#[test]
fn a_document_that_is_no_set_of_answers_is_an_error_and_not_an_answer() {
    let deltas = answered(&verdict(), &serde_json::json!({ "message": "overloaded" }));
    assert_eq!(deltas, vec![Delta::Stop(StopReason::Error)]);
}

#[test]
fn the_hints_written_for_this_protocol_reach_no_provider_that_would_refuse_them() {
    // A strict schema checker answers 400 to a keyword it does not know.
    for name in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
    ] {
        let body = adapter(name).request(&plain_model(), &plain_context(), &verdict());
        let sent = body.to_string();
        assert!(
            !sent.contains("x-criteria") && !sent.contains("x-from"),
            "{name}: {sent}"
        );
        assert!(
            sent.contains("download-execute"),
            "{name}: the schema itself still goes"
        );
    }
}
