use super::{Catalog, assemble};
use crate::mind::lua::adapter::LuaAdapter;
use crate::mind::lua::engine::Engine;
use crate::mind::model::{Context, Message};
use crate::mind::provider::api::{Adapter, Options};
use crate::mind::provider::model::Api;
use crate::scratch::Scratch;
use serde_json::json;

fn source(extra: &str) -> String {
    format!(
        r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id, api="openai-completions", auth={{kind="none"}}}})
        end
        {extra}
    "#
    )
}

#[test]
fn lua_catalog_preserves_model_protocols_and_dispatches_the_selected_adapter() {
    let dir = Scratch::new("melchior-catalog", "mixed-protocol");
    std::fs::write(dir.join("providers.lua"), source(r#"
        melchior.provider("mixed", {name="Mixed", api="anthropic-messages",
            base_url="https://fixture.invalid", auth={kind="none"}, models={
                {id="inherit",name="Inherited",context_window=8192,max_tokens=128},
                {id="chat",name="Chat",api="openai-completions",context_window=8192,max_tokens=128},
                {id="responses",name="Responses",api="openai-responses",context_window=8192,max_tokens=128}
            }})
    "#)).expect("declarations");
    let catalog = Catalog::load(&dir).expect("catalog");
    let context = Context {
        messages: vec![Message::user("question")],
        ..Context::default()
    };
    for (id, api, field) in [
        ("inherit", Api::AnthropicMessages, "messages"),
        ("chat", Api::OpenAiCompletions, "messages"),
        ("responses", Api::OpenAiResponses, "input"),
    ] {
        let (_, model) = catalog
            .find(&format!("mixed/{id}"))
            .expect("declared model");
        assert_eq!(model.api, api);
        let card = catalog
            .cards()
            .into_iter()
            .find(|card| card.id == format!("mixed/{id}"))
            .expect("card");
        assert_eq!(card.api, api.as_str());
        let mut engine = Engine::new();
        engine.run(super::APIS, "apis.lua").expect("protocols");
        let adapter = LuaAdapter::new(engine, model.api.as_str()).expect("model dispatch");
        let body = adapter.request(model, &context, &Options::default());
        assert!(body[field].is_array());
        assert_eq!(body["model"], id);
    }
}

#[test]
fn malformed_provider_is_diagnosed_instead_of_disappearing() {
    let dir = Scratch::new("melchior-catalog", "invalid-provider");
    std::fs::write(
        dir.join("providers.lua"),
        source(
            r#"
        melchior.provider("broken", {name="Broken", api="secret-value", auth={kind="none"}})
    "#,
        ),
    )
    .expect("declarations");
    let error = match Catalog::load(&dir) {
        Ok(_) => panic!("invalid provider was accepted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("broken") && error.contains("api"));
    assert!(!error.contains("secret-value"));
}

#[test]
fn invalid_values_are_redacted_and_the_offending_field_is_identified() {
    let valid = json!({"name":"Fixture","api":"openai-completions","auth":{"kind":"none"}});
    for (field, value, location) in [
        ("api", json!("secret-protocol"), "api"),
        ("auth", json!({"kind":"secret-auth"}), "auth"),
        (
            "models",
            json!([{"id":"m","api":"secret-model-protocol"}]),
            "models[1].api",
        ),
        ("models", json!([{"id":"m","api":null}]), "models[1].api"),
    ] {
        let mut declaration = valid.clone();
        declaration[field] = value;
        let error = assemble("broken", &declaration)
            .expect_err("invalid declaration")
            .to_string();
        assert!(
            error.contains("broken") && error.contains(location),
            "{error}"
        );
        assert!(!error.contains("secret-"));
    }
    assert!(
        assemble("broken", &json!("secret-table"))
            .expect_err("not a table")
            .to_string()
            .contains("declaration table")
    );
}

#[test]
fn invalid_header_and_origin_declarations_are_attributed_without_values() {
    let valid = json!({"name":"Fixture","api":"openai-completions","auth":{"kind":"none"}});
    for (field, value, location) in [
        (
            "headers",
            json!({"Authorization":"private-sentinel"}),
            "headers",
        ),
        (
            "headers",
            json!({"x-private-sentinel":"bad\r\nvalue"}),
            "headers",
        ),
        (
            "credential_origins",
            json!(["https://private-sentinel@fixture.invalid"]),
            "credential_origins",
        ),
        (
            "models",
            json!([{"id":"m","name":"M","context_window":8192,"max_tokens":128,
            "headers":{"x-api-key":"private-sentinel"}}]),
            "models[1]",
        ),
    ] {
        let mut declaration = valid.clone();
        declaration[field] = value;
        let error = assemble("broken", &declaration)
            .expect_err("invalid request configuration")
            .to_string();
        assert!(
            error.contains("broken") && error.contains(location),
            "{error}"
        );
        assert!(!error.contains("private-sentinel"));
    }
}

#[test]
fn lua_cost_tiers_and_explicit_zero_prices_survive_catalog_loading() {
    let dir = Scratch::new("melchior-catalog", "tiered-prices");
    std::fs::write(dir.join("providers.lua"),source(r#"
        melchior.provider("prices",{name="Prices",api="openai-completions",auth={kind="none"},models={
            {id="tiered",name="Tiered",context_window=8192,max_tokens=128,
             cost={input=2,output=4,tiers={{input_tokens_above=10,input=3,output=6}}}},
            {id="free",name="Free",context_window=8192,max_tokens=128,cost={input=0,output=0}},
            {id="unknown",name="Unknown",context_window=8192,max_tokens=128}
        }})
    "#)).expect("declarations");
    let catalog = Catalog::load(&dir).expect("catalog");
    let (_, model) = catalog.find("prices/tiered").expect("model");
    for (input, expected) in [(10, 24.0), (11, 39.0)] {
        let usage = crate::mind::model::Usage {
            input,
            output: 1,
            ..Default::default()
        };
        assert!((usage.price(&model.cost) * 1_000_000.0 - expected).abs() < 1e-9);
    }
    let cards = catalog.cards();
    assert_eq!(
        cards
            .iter()
            .find(|card| card.id == "prices/free")
            .expect("free card")
            .pricing,
        Some(crate::mind::model::Pricing {
            input: Some("0".into()),
            output: Some("0".into())
        })
    );
    assert!(
        cards
            .iter()
            .find(|card| card.id == "prices/unknown")
            .expect("unknown card")
            .pricing
            .is_none()
    );
}

#[test]
fn invalid_cost_schedules_are_attributed_without_echoing_values() {
    for cost in [
        json!({"input":"secret-price"}),
        json!({"input":-1}),
        json!({"tiers":[{"input_tokens_above":10},{"input_tokens_above":10}]}),
        json!({"tiers":[{"input_tokens_above":10,"input":"secret-price"}]}),
    ] {
        let raw = json!({"name":"Broken","api":"openai-completions","auth":{"kind":"none"},
            "models":[{"id":"one","name":"One","context_window":8192,"max_tokens":128,"cost":cost}]});
        let why = assemble("prices", &raw)
            .expect_err("invalid costs")
            .to_string();
        assert!(
            why.contains("prices") && why.contains("models[1].cost"),
            "{why}"
        );
        assert!(!why.contains("secret-price"));
    }
}
