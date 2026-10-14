//! What a config cannot reach. `Lua::full()` installs the whole standard library and this takes
//! pieces back out, so the list below is what must not be reachable rather than a hand-assembled
//! list of what may be.

use luna::{Callback, CallbackReturn, Context, Function, Lua, Value};

/// Globals a config must not have: `os.execute` spawns, `os.remove`, `os.rename` and
/// `os.tmpname` write outside the `Ops` seam where path checking lives, `os.exit` would let
/// a config file end the daemon, and `string.dump` makes the bytecode nothing here loads.
const REMOVED: &[(&str, &str)] = &[
    ("os", "execute"),
    ("os", "exit"),
    ("os", "remove"),
    ("os", "rename"),
    ("os", "tmpname"),
    ("os", "setlocale"),
    ("string", "dump"),
];

/// Globals removed entirely: `io` opens files and `io.popen` spawns, and the loaders read
/// arbitrary files.
const REMOVED_TABLES: &[&str] = &["io", "package", "dofile", "loadfile", "require"];

/// The first byte of a precompiled chunk: PUC-Rio's `\x1bLua` and luna's `\x1bLuna` alike.
const PRECOMPILED: u8 = 0x1b;

/// What a refused precompiled chunk is told.
pub const REFUSED_PRECOMPILED: &str = "a precompiled chunk is refused; only source runs";

/// Whether a chunk is bytecode rather than source.
#[must_use]
pub fn precompiled(chunk: &[u8]) -> bool {
    chunk.first() == Some(&PRECOMPILED)
}

/// Take away what a config must not be able to do.
pub fn apply(lua: &mut Lua) {
    lua.enter(|ctx| {
        for (table, field) in REMOVED {
            if let Value::Table(t) = ctx.get_global_value(table) {
                t.set(ctx, *field, Value::Nil).ok();
            }
        }
        for name in REMOVED_TABLES {
            ctx.set_global(name, Value::Nil);
        }
        guard_load(ctx);
    });
}

/// Replace `load` with one that refuses bytecode in every mode and hands source to the original.
fn guard_load(ctx: Context<'_>) {
    let Ok(load) = ctx.get_global::<Function>("load") else {
        return;
    };
    let guarded = Callback::from_fn_with(&ctx, load, |load, ctx, _exec, stack| {
        if let Value::String(chunk) = stack.get(0) {
            if precompiled(chunk.as_bytes()) {
                return Err(refusal(ctx, &format!("load: {REFUSED_PRECOMPILED}")));
            }
        }
        Ok(CallbackReturn::Call {
            function: *load,
            then: None,
        })
    });
    ctx.set_global("load", guarded);
}

/// An error raised into Lua as a string.
fn refusal<'gc>(ctx: Context<'gc>, message: &str) -> luna::Error<'gc> {
    luna::Error::from_value(Value::String(ctx.intern(message.as_bytes())))
}

#[cfg(test)]
mod tests {
    use crate::mind::lua::engine::Engine;

    /// What one expression evaluates to inside a fresh engine.
    fn probe(expression: &str) -> String {
        let mut engine = Engine::new();
        engine
            .run(
                &format!("melchior.answer = tostring({expression})"),
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .unwrap_or("<absent>")
            .to_owned()
    }

    #[test]
    fn a_config_cannot_spawn_a_process() {
        assert_eq!(probe("os.execute"), "nil");
        assert_eq!(probe("io"), "nil");
    }

    #[test]
    fn a_config_cannot_write_outside_the_ops_seam() {
        for expression in ["os.remove", "os.rename", "os.tmpname"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn a_config_cannot_end_the_daemon() {
        assert_eq!(probe("os.exit"), "nil");
    }

    #[test]
    fn a_config_cannot_load_arbitrary_files() {
        for expression in ["dofile", "loadfile", "require", "package"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn what_a_config_legitimately_needs_still_works() {
        assert_ne!(probe("os.getenv"), "nil", "reading the environment is fine");
        assert_ne!(probe("os.time"), "nil");
        assert_ne!(
            probe("load"),
            "nil",
            "the family's clients are loaded chunks"
        );
        assert_ne!(probe("string.format"), "nil");
        assert_ne!(probe("table.concat"), "nil");
        assert_ne!(probe("melchior.json.encode"), "nil");
        assert_ne!(probe("melchior.stream.connect"), "nil");
    }
}
