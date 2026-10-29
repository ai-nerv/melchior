use super::Engine;
use crate::mind::lua::convert::{json_from_lua, shape};
use luna::{UserData, Value};
use serde_json::{Value as Json, json};

fn evaluate(engine: &mut Engine, source: &str) -> Json {
    engine.run(source, "json-contract.lua").expect("runs");
    engine
        .lua
        .enter(|ctx| json_from_lua(ctx, ctx.get_global_value("result")).expect("JSON result"))
}

#[test]
fn constructors_preserve_empty_containers_null_positions_and_identity() {
    let mut engine = Engine::new();
    let result = evaluate(
        &mut engine,
        r#"
        local j = melchior.json
        local array, object = {}, {}
        assert(j.array(array) == array and j.object(object) == object)
        array[1], array[2], array[3] = j.null, j.array(), j.object()
        result = {array = array, object = object, null = j.null,
                  object_with_numeric_key = j.object({[1] = 'one'})}
        "#,
    );
    assert_eq!(
        result,
        json!({"array":[null,[],{}],"object":{},"null":null,
        "object_with_numeric_key":{"1":"one"}})
    );
}

#[test]
fn typed_arrays_sort_integer_entries_and_refuse_holes_or_noninteger_keys() {
    let mut engine = Engine::new();
    let result = evaluate(
        &mut engine,
        r#"
        local j = melchior.json
        local a = j.array()
        for i = 80, 1, -1 do a[i] = i end
        assert(j.encode(a) ~= nil)
        local sparse, mixed, zero, negative, fraction = j.array(), j.array(), j.array(), j.array(), j.array()
        sparse[2], mixed.x, zero[0], negative[-1], fraction[1.5] = 1, 1, 1, 1, 1
        result = {ordered = a, sparse = j.encode(sparse) == nil, mixed = j.encode(mixed) == nil,
                  zero = j.encode(zero) == nil, negative = j.encode(negative) == nil,
                  fraction = j.encode(fraction) == nil}
        "#,
    );
    assert_eq!(result["ordered"], json!((1..=80).collect::<Vec<_>>()));
    for key in ["sparse", "mixed", "zero", "negative", "fraction"] {
        assert_eq!(result[key], true, "{key}");
    }
}

#[test]
fn typed_tables_keep_cycle_refusal_and_allow_shared_values() {
    let mut engine = Engine::new();
    let result = evaluate(
        &mut engine,
        r#"
        local j = melchior.json
        local a, b = j.array(), j.object()
        a[1], b.back = b, a
        assert(j.encode(a) == nil)
        b.back = nil
        result = j.array({b, b, j.null})
        "#,
    );
    assert_eq!(result, json!([{}, {}, null]));
}

#[test]
fn constructors_do_not_replace_custom_metatables_or_run_table_metamethods() {
    let mut engine = Engine::new();
    let result = evaluate(
        &mut engine,
        r#"
        local j = melchior.json
        local a = setmetatable({}, {__pairs = function() error('executed pairs') end,
                                   __len = function() error('executed len') end})
        local ok = pcall(j.array, a)
        assert(not ok and j.encode(a) == '{}')
        assert(not pcall(j.object, 1))
        local protected = j.array()
        assert(not pcall(setmetatable, protected, {}))
        result = protected
        "#,
    );
    assert_eq!(result, json!([]));
}

#[test]
fn only_the_private_null_userdata_encodes_as_null() {
    let mut engine = Engine::new();
    engine.lua.enter(|ctx| {
        ctx.set_global("unrelated", UserData::new_static(&ctx, ()));
    });
    let result = evaluate(
        &mut engine,
        r#"
        local j = melchior.json
        result = {unknown = j.encode(unrelated) == nil,
                  shape = j.encode(getmetatable(j.array())) == nil,
                  null = j.encode(j.null)}
        "#,
    );
    assert_eq!(result, json!({"unknown":true,"shape":true,"null":"null"}));
}

#[test]
fn typed_values_and_constructors_survive_gc_without_finalizers() {
    let mut engine = Engine::new();
    evaluate(
        &mut engine,
        "result = melchior.json.array({melchior.json.null, melchior.json.object()})",
    );
    for _ in 0..4 {
        engine.lua.gc_collect();
    }
    assert_eq!(
        evaluate(&mut engine, "assert(result[1] == melchior.json.null)"),
        json!([null, {}])
    );
    engine.lua.enter(|ctx| {
        let types = ctx.fetch(&engine.json_types);
        for key in ["array", "object"] {
            let Value::Table(meta) = types.get_value(ctx, key) else {
                panic!("missing shape")
            };
            assert!(meta.get_value(ctx, "__gc").is_nil());
        }
        let Value::UserData(null) = types.get_value(ctx, "null") else {
            panic!("missing null")
        };
        assert!(null.metatable().is_none());
    });
    assert_eq!(
        evaluate(&mut engine, "result = melchior.json.array()"),
        json!([])
    );
}

#[test]
fn exact_request_payloads_preserve_scalars_and_nested_literal_metadata_keys() {
    let mut engine = Engine::new();
    engine.run(r#"
        melchior.api('echo', {request = function(model, ctx, opts)
            assert(model.optional == nil and opts.optional == nil)
            local p, a, s = ctx.tools[1].parameters, ctx.messages[1].content[1].arguments, opts.schema.schema
            assert(p == melchior.json.null or p ~= nil)
            return {parameters = p, arguments = a, schema = s}
        end})
    "#, "echo.lua").expect("register");
    for value in [
        json!(null),
        json!(false),
        json!(0),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(0.125),
        json!(""),
        json!([]),
        json!({}),
        json!({"__metatable":null,"cache_control":[null,[],{}],"__melchior_json_types":{}}),
    ] {
        let context = json!({"tools":[{"parameters":value}], "messages":[{"content":[{"type":"tool_call","arguments":value}]}]});
        let options = json!({"schema":{"schema":value},"optional":null});
        let result = engine.call_api(
            "echo",
            "request",
            &[json!({"optional":null}), context, options],
        );
        assert_eq!(
            result,
            Some(json!({"parameters":value,"arguments":value,"schema":value}))
        );
    }
}

#[test]
fn optional_metadata_decode_and_nonrequest_calls_keep_nil_semantics() {
    let mut engine = Engine::new();
    engine.run(r#"
        melchior.api('legacy', {
            headers = function(key) return {nil_key = key == nil} end,
            on_event = function(state)
                local scratch = state.scratch or {}
                assert(melchior.json.decode('null') == nil)
                assert(melchior.json.decode('{"optional":null}').optional == nil)
                return {scratch_is_nil = state.scratch == nil, scratch_is_table = type(scratch) == 'table'}
            end
        })
    "#, "legacy.lua").expect("register");
    assert_eq!(
        engine.call_api("legacy", "headers", &[Json::Null]),
        Some(json!({"nil_key":true}))
    );
    assert_eq!(
        engine.call_api("legacy", "on_event", &[json!({"scratch":null})]),
        Some(json!({"scratch_is_nil":true,"scratch_is_table":true}))
    );
}

#[test]
fn tagged_tables_still_obey_depth_node_and_string_budgets() {
    for (source, reason) in [
        (
            "local t = j.array() for _ = 1, 40 do t = j.array({t}) end",
            "deeper than 32",
        ),
        (
            "local t = 0 for _ = 1, 20 do t = j.array({t,t,t,t}) end",
            "values",
        ),
        (
            "local s = ('x'):rep(1 << 20) local t = j.array() for i = 1, 65 do t[i] = s end",
            "bytes of strings",
        ),
    ] {
        let mut engine = Engine::new();
        let result = evaluate(
            &mut engine,
            &format!(
                "local j = melchior.json {source} local ok, why = pcall(melchior.provider, 'p', t) \
             result = {{refused = not ok, reason = tostring(why), encode_nil = j.encode(t) == nil}}"
            ),
        );
        assert_eq!(result["refused"], true);
        assert_eq!(result["encode_nil"], true);
        assert!(
            result["reason"].as_str().expect("reason").contains(reason),
            "{result}"
        );
    }
}

#[test]
fn shape_metadata_is_not_traversed_as_payload() {
    let mut engine = Engine::new();
    engine.lua.enter(|ctx| {
        let types = ctx.fetch(&engine.json_types);
        let Value::Table(meta) = types.get_value(ctx, "array") else {
            panic!("array meta")
        };
        meta.set(ctx, "cycle", meta).expect("cycle metadata");
        let table = luna::Table::new(&ctx);
        shape::set(ctx, types, table, shape::Shape::Array);
        assert_eq!(json_from_lua(ctx, Value::Table(table)), Ok(json!([])));
    });
}
