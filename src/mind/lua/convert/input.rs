use super::{lua_from_json, shape};
use luna::{Context, Table, Value};
use serde_json::Value as Json;

#[derive(Clone, Copy)]
enum Boundary {
    Ordinary,
    Context,
    Tools,
    Tool,
    Messages,
    Message,
    Contents,
    Content,
    Options,
    Schema,
    Exact,
}

impl Boundary {
    fn field(self, key: &str) -> Self {
        match (self, key) {
            (Self::Context, "tools") => Self::Tools,
            (Self::Context, "messages") => Self::Messages,
            (Self::Tool, "parameters")
            | (Self::Content, "arguments")
            | (Self::Schema, "schema") => Self::Exact,
            (Self::Message, "content") => Self::Contents,
            (Self::Options, "schema") => Self::Schema,
            (Self::Exact, _) => Self::Exact,
            _ => Self::Ordinary,
        }
    }

    fn item(self) -> Self {
        match self {
            Self::Tools => Self::Tool,
            Self::Messages => Self::Message,
            Self::Contents => Self::Content,
            Self::Exact => Self::Exact,
            _ => Self::Ordinary,
        }
    }
}

pub(in crate::mind::lua) fn api_args<'gc>(
    ctx: Context<'gc>,
    types: Table<'gc>,
    method: &str,
    args: &[Json],
) -> Value<'gc> {
    let table = Table::new(&ctx);
    for (index, value) in args.iter().enumerate() {
        let boundary = match (method, index) {
            ("request", 1) => Boundary::Context,
            ("request", 2) => Boundary::Options,
            _ => Boundary::Ordinary,
        };
        table
            .set(ctx, index as i64 + 1, convert(ctx, types, value, boundary))
            .ok();
    }
    Value::Table(table)
}

fn convert<'gc>(
    ctx: Context<'gc>,
    types: Table<'gc>,
    value: &Json,
    boundary: Boundary,
) -> Value<'gc> {
    if matches!(boundary, Boundary::Ordinary) {
        return lua_from_json(ctx, value);
    }
    let exact = matches!(boundary, Boundary::Exact);
    match value {
        Json::Null if exact => types.get_value(ctx, "null"),
        Json::Number(number) if exact && number.as_i64().is_none() => number.as_u64().map_or_else(
            || lua_from_json(ctx, value),
            |number| shape::unsigned(ctx, number),
        ),
        Json::Array(items) => {
            let table = Table::new(&ctx);
            for (index, item) in items.iter().enumerate() {
                table
                    .set(
                        ctx,
                        index as i64 + 1,
                        convert(ctx, types, item, boundary.item()),
                    )
                    .ok();
            }
            if exact {
                shape::set(ctx, types, table, shape::Shape::Array);
            }
            Value::Table(table)
        }
        Json::Object(fields) => {
            let table = Table::new(&ctx);
            for (key, item) in fields {
                let name = luna::String::from_slice(&ctx, key.as_bytes());
                table
                    .set(ctx, name, convert(ctx, types, item, boundary.field(key)))
                    .ok();
            }
            if exact {
                shape::set(ctx, types, table, shape::Shape::Object);
            }
            Value::Table(table)
        }
        _ => lua_from_json(ctx, value),
    }
}
