use luna::{Context, Table, UserData, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::mind::lua) enum Shape {
    Array,
    Object,
}

struct Null;

struct Unsigned(u64);

pub(in crate::mind::lua) fn types(ctx: Context<'_>) -> Table<'_> {
    let types = Table::new(&ctx);
    for (name, shape) in [("array", Shape::Array), ("object", Shape::Object)] {
        let meta = Table::new(&ctx);
        meta.set(ctx, "__metatable", UserData::new_static(&ctx, shape))
            .ok();
        types.set(ctx, name, meta).ok();
    }
    types
        .set(ctx, "null", UserData::new_static(&ctx, Null))
        .ok();
    types
}

pub(in crate::mind::lua) fn get<'gc>(ctx: Context<'gc>, table: Table<'gc>) -> Option<Shape> {
    let Value::UserData(value) = table.metatable()?.get_value(ctx, "__metatable") else {
        return None;
    };
    value.downcast_static::<Shape>().ok().copied()
}

pub(in crate::mind::lua) fn set<'gc>(
    ctx: Context<'gc>,
    types: Table<'gc>,
    table: Table<'gc>,
    shape: Shape,
) {
    let name = match shape {
        Shape::Array => "array",
        Shape::Object => "object",
    };
    if let Value::Table(meta) = types.get_value(ctx, name) {
        table.set_metatable(ctx, Some(meta));
    }
}

pub(super) fn unsigned(ctx: Context<'_>, value: u64) -> Value<'_> {
    Value::UserData(UserData::new_static(&ctx, Unsigned(value)))
}

pub(super) fn scalar(value: UserData<'_>) -> Option<serde_json::Value> {
    if value.is_static::<Null>() {
        Some(serde_json::Value::Null)
    } else {
        value
            .downcast_static::<Unsigned>()
            .ok()
            .map(|value| serde_json::Value::Number(value.0.into()))
    }
}
