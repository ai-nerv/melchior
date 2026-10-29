//! Turning what the config declared into owned Rust values. luna's `Value<'gc>` cannot leave
//! `lua.enter`, so everything above this crate sees `serde_json::Value` instead.

use luna::{Table, Value};
use serde_json::{Map, Number};
use std::collections::HashSet;

mod input;
pub(super) mod shape;

pub(super) use input::api_args;

/// How deep one conversion may nest.
const DEPTH: usize = 32;

/// How many values one conversion may visit: the root and every entry of every table.
pub const NODES: usize = 1 << 21;

/// How many bytes of strings, keys included, one conversion may copy.
pub const BYTES: usize = 4 * crate::framing::MOST;

/// Why a Lua value has no JSON form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Undescribable {
    #[error("a {0} has no JSON form")]
    Kind(&'static str),
    #[error("a table key is neither a string nor an integer")]
    Key,
    #[error("a JSON array must have contiguous integer keys starting at 1")]
    Array,
    #[error("tables nest deeper than {DEPTH}")]
    Deep,
    #[error("a table contains itself")]
    Cycle,
    #[error("more than {NODES} values")]
    Wide,
    #[error("more than {BYTES} bytes of strings")]
    Long,
}

/// A Lua value as JSON. Typed containers retain their shape; untyped nonempty tables with
/// consecutive integer keys become arrays and other untyped tables become objects.
pub fn json_from_lua<'gc>(
    ctx: luna::Context<'gc>,
    value: Value<'gc>,
) -> Result<serde_json::Value, Undescribable> {
    let mut walk = Walk {
        ctx,
        nodes: 0,
        bytes: 0,
        open: HashSet::new(),
    };
    walk.spend(1, 0)?;
    walk.value(value, 0)
}

/// One conversion: what it has spent, and the tables it is inside.
struct Walk<'gc> {
    ctx: luna::Context<'gc>,
    nodes: usize,
    bytes: usize,
    open: HashSet<Table<'gc>>,
}

impl<'gc> Walk<'gc> {
    fn spend(&mut self, nodes: usize, bytes: usize) -> Result<(), Undescribable> {
        self.nodes = self.nodes.saturating_add(nodes);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.nodes > NODES {
            return Err(Undescribable::Wide);
        }
        if self.bytes > BYTES {
            return Err(Undescribable::Long);
        }
        Ok(())
    }

    fn string(&mut self, s: luna::String<'gc>) -> Result<String, Undescribable> {
        self.spend(0, s.as_bytes().len())?;
        Ok(String::from_utf8_lossy(s.as_bytes()).into_owned())
    }

    fn value(
        &mut self,
        value: Value<'gc>,
        depth: usize,
    ) -> Result<serde_json::Value, Undescribable> {
        if depth > DEPTH {
            return Err(Undescribable::Deep);
        }
        Ok(match value {
            Value::Nil => serde_json::Value::Null,
            Value::Boolean(b) => serde_json::Value::Bool(b),
            Value::Integer(i) => serde_json::Value::Number(i.into()),
            Value::Number(f) => Number::from_f64(f).map_or(serde_json::Value::Null, Into::into),
            Value::String(s) => serde_json::Value::String(self.string(s)?),
            Value::Table(t) => self.table(t, depth)?,
            Value::UserData(value) => {
                shape::scalar(value).ok_or(Undescribable::Kind("userdata"))?
            }
            // A function belongs to the VM that made it; registrars keep those separately.
            other => return Err(Undescribable::Kind(other.type_name())),
        })
    }

    fn table(
        &mut self,
        table: Table<'gc>,
        depth: usize,
    ) -> Result<serde_json::Value, Undescribable> {
        if !self.open.insert(table) {
            return Err(Undescribable::Cycle);
        }
        let mut entries: Vec<(Value<'gc>, Value<'gc>)> = Vec::new();
        for entry in table.iter(self.ctx) {
            self.spend(1, 0)?;
            entries.push(entry);
        }

        let shape = shape::get(self.ctx, table);
        if shape == Some(shape::Shape::Array) {
            entries.sort_unstable_by_key(|(key, _)| match key {
                Value::Integer(index) => *index,
                _ => i64::MIN,
            });
        }
        let consecutive = entries
            .iter()
            .enumerate()
            .all(|(index, (key, _))| matches!(key, Value::Integer(i) if *i == index as i64 + 1));
        let is_list = match shape {
            Some(shape::Shape::Array) if !consecutive => return Err(Undescribable::Array),
            Some(shape::Shape::Array) => true,
            Some(shape::Shape::Object) => false,
            None => !entries.is_empty() && consecutive,
        };

        let out = if is_list {
            let mut out = Vec::with_capacity(entries.len());
            for (_, value) in entries {
                out.push(self.value(value, depth + 1)?);
            }
            serde_json::Value::Array(out)
        } else {
            let mut out = Map::new();
            for (key, value) in entries {
                let name = match key {
                    Value::String(s) => self.string(s)?,
                    Value::Integer(i) => i.to_string(),
                    // A key that is not a name refuses the whole table rather than being skipped.
                    _ => return Err(Undescribable::Key),
                };
                out.insert(name, self.value(value, depth + 1)?);
            }
            serde_json::Value::Object(out)
        };
        self.open.remove(&table);
        Ok(out)
    }
}

/// Read a declaration out of a Lua table.
pub trait FromLua: Sized {
    fn from_json(value: serde_json::Value) -> Result<Self, String>;
}

impl<T: serde::de::DeserializeOwned> FromLua for T {
    fn from_json(value: serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
}

/// A JSON control value as Lua. Nulls become nil; arrays and objects become untyped tables.
pub fn lua_from_json<'gc>(ctx: luna::Context<'gc>, value: &serde_json::Value) -> Value<'gc> {
    match value {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Number(n.as_f64().unwrap_or(0.0)), Value::Integer),
        serde_json::Value::String(s) => Value::String(luna::String::from_slice(&ctx, s.as_bytes())),
        serde_json::Value::Array(items) => {
            let table = Table::new(&ctx);
            for (index, item) in items.iter().enumerate() {
                table
                    .set(ctx, index as i64 + 1, lua_from_json(ctx, item))
                    .ok();
            }
            Value::Table(table)
        }
        serde_json::Value::Object(fields) => {
            let table = Table::new(&ctx);
            for (key, item) in fields {
                let key = luna::String::from_slice(&ctx, key.as_bytes());
                table.set(ctx, key, lua_from_json(ctx, item)).ok();
            }
            Value::Table(table)
        }
    }
}

/// A conversion ends in a named refusal, never a hang or an abort, however the table is built.
#[cfg(test)]
mod tests {
    use super::{BYTES, NODES};
    use crate::mind::lua::engine::Engine;

    /// What a chunk left in `melchior.said`, run in a fresh engine.
    fn said(source: &str) -> String {
        let mut engine = Engine::new();
        engine.run(source, "convert.lua").expect("runs");
        engine.harvest();
        engine
            .config()
            .string("said")
            .unwrap_or("<absent>")
            .to_owned()
    }

    /// The same chunk handed to a registrar, which raises, and to `json.encode`, which answers nil.
    fn refused(build: &str) -> String {
        said(&format!(
            "{build}\n\
             local ok, why = pcall(melchior.provider, 'p', t)\n\
             melchior.said = tostring(ok) .. ' ' .. tostring(why) .. ' ' .. \
             tostring(melchior.json.encode(t))"
        ))
    }

    #[test]
    fn a_table_that_contains_itself_is_refused_by_name() {
        let said = refused("local t = { n = 1 } t.again = { t }");
        assert!(said.starts_with("false "), "{said}");
        assert!(said.contains("a table contains itself"), "{said}");
        assert!(said.ends_with(" nil"), "{said}");
    }

    #[test]
    fn a_table_shared_twice_is_not_a_cycle() {
        let said =
            said("local s = { 1, 2 } melchior.said = melchior.json.encode({ a = s, b = { s } })");
        let back: serde_json::Value = serde_json::from_str(&said).expect(&said);
        assert_eq!(back, serde_json::json!({ "a": [1, 2], "b": [[1, 2]] }));
    }

    #[test]
    fn a_fan_out_past_the_node_budget_is_refused() {
        let said = refused("local t = 0 for _ = 1, 20 do t = { t, t, t, t } end");
        assert!(
            said.contains(&format!("more than {NODES} values")),
            "{said}"
        );
        assert!(said.ends_with(" nil"), "{said}");
    }

    #[test]
    fn strings_past_the_byte_budget_are_refused() {
        let said =
            refused("local s = ('x'):rep(1 << 20) local t = {} for i = 1, 65 do t[i] = s end");
        assert!(said.contains(&format!("more than {BYTES} bytes")), "{said}");
        assert!(said.ends_with(" nil"), "{said}");
    }

    #[test]
    fn the_depth_cap_still_holds() {
        let said = refused("local t = {} for _ = 1, 40 do t = { t } end");
        assert!(said.contains("deeper than 32"), "{said}");
    }
}
