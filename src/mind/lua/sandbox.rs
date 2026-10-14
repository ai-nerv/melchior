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

/// Globals removed entirely: `io` opens files and `io.popen` spawns, the loaders read
/// arbitrary files, and `debug` rewrites other frames and attaches metatables past
/// `setmetatable`.
const REMOVED_TABLES: &[&str] = &["io", "package", "dofile", "loadfile", "require", "debug"];

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
        guard_setmetatable(ctx);
    });
}

/// Replace `setmetatable` with one that refuses a metatable carrying `__gc`, so nothing is ever
/// queued for a finalizer.
fn guard_setmetatable(ctx: Context<'_>) {
    let Ok(original) = ctx.get_global::<Function>("setmetatable") else {
        return;
    };
    let guarded = Callback::from_fn_with(&ctx, original, |original, ctx, _exec, stack| {
        if let Value::Table(metatable) = stack.get(1)
            && !metatable.get_value(ctx, "__gc").is_nil()
        {
            return Err(refusal(ctx, REFUSED_GC));
        }
        Ok(CallbackReturn::Call {
            function: *original,
            then: None,
        })
    });
    ctx.set_global("setmetatable", guarded);
}

/// What a metatable carrying `__gc` is told.
const REFUSED_GC: &str = "setmetatable: a metatable with __gc is refused; no finalizer runs here";

/// Replace `load` with one that refuses bytecode in every mode and hands source to the original.
fn guard_load(ctx: Context<'_>) {
    let Ok(load) = ctx.get_global::<Function>("load") else {
        return;
    };
    let guarded = Callback::from_fn_with(&ctx, load, |load, ctx, _exec, stack| {
        if let Value::String(chunk) = stack.get(0)
            && precompiled(chunk.as_bytes())
        {
            return Err(refusal(ctx, &format!("load: {REFUSED_PRECOMPILED}")));
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

    /// Every global a config sees, and every field of every global table, sorted.
    const SURFACE: &str = "local names = {}\n\
        for name, value in pairs(_G) do\n\
          names[#names + 1] = name\n\
          if type(value) == 'table' and value ~= _G then\n\
            for field in pairs(value) do names[#names + 1] = name .. '.' .. tostring(field) end\n\
          end\n\
        end\n\
        table.sort(names)\n\
        return table.concat(names, ' ')";

    /// What a fresh engine lends, and nothing else: an addition fails here until it is listed.
    const ALLOWED: &[&str] = &[
        // base
        "_G",
        "_VERSION",
        "assert",
        "collectgarbage",
        "error",
        "getmetatable",
        "ipairs",
        "load",
        "next",
        "pairs",
        "pcall",
        "print",
        "rawequal",
        "rawget",
        "rawlen",
        "rawset",
        "select",
        "setmetatable",
        "tonumber",
        "tostring",
        "type",
        "warn",
        "xpcall",
        // coroutine
        "coroutine",
        "coroutine.close",
        "coroutine.continue",
        "coroutine.create",
        "coroutine.isyieldable",
        "coroutine.resume",
        "coroutine.running",
        "coroutine.status",
        "coroutine.wrap",
        "coroutine.yield",
        "coroutine.yieldto",
        // math
        "math",
        "math.abs",
        "math.acos",
        "math.asin",
        "math.atan",
        "math.ceil",
        "math.cos",
        "math.deg",
        "math.exp",
        "math.floor",
        "math.fmod",
        "math.huge",
        "math.log",
        "math.max",
        "math.maxinteger",
        "math.min",
        "math.mininteger",
        "math.modf",
        "math.pi",
        "math.rad",
        "math.random",
        "math.randomseed",
        "math.sin",
        "math.sqrt",
        "math.tan",
        "math.tointeger",
        "math.type",
        "math.ult",
        // os, trimmed to reading the clock and the environment
        "os",
        "os.clock",
        "os.date",
        "os.difftime",
        "os.getenv",
        "os.time",
        // string
        "string",
        "string.byte",
        "string.char",
        "string.find",
        "string.format",
        "string.gmatch",
        "string.gsub",
        "string.len",
        "string.lower",
        "string.match",
        "string.pack",
        "string.packsize",
        "string.rep",
        "string.reverse",
        "string.sub",
        "string.unpack",
        "string.upper",
        // table
        "table",
        "table.clear",
        "table.concat",
        "table.freeze",
        "table.insert",
        "table.interceptall",
        "table.isfrozen",
        "table.move",
        "table.pack",
        "table.remove",
        "table.sort",
        "table.unpack",
        // utf8
        "utf8",
        "utf8.char",
        "utf8.charpattern",
        "utf8.codepoint",
        "utf8.codes",
        "utf8.len",
        "utf8.offset",
        // melchior's own module, and the stream a family client finds without it
        "__melchior_apis",
        "__stream",
        "__stream.connect",
        "melchior",
        "melchior.api",
        "melchior.apis",
        "melchior.fs",
        "melchior.json",
        "melchior.load",
        "melchior.provider",
        "melchior.self",
        "melchior.stream",
        "melchior.ui",
    ];

    #[test]
    fn the_surface_is_exactly_the_allowlist() {
        let surface = probe(&format!("(function() {SURFACE} end)()"));
        let seen: Vec<&str> = surface.split(' ').collect();
        let extra: Vec<&&str> = seen.iter().filter(|name| !ALLOWED.contains(name)).collect();
        let missing: Vec<&&str> = ALLOWED.iter().filter(|name| !seen.contains(name)).collect();
        assert!(extra.is_empty(), "lent and not allowed: {extra:?}");
        assert!(
            missing.is_empty(),
            "allowed and no longer lent: {missing:?}"
        );
        for name in [
            "debug",
            "loadfile",
            "dofile",
            "io",
            "os.execute",
            "package",
            "package.loadlib",
            "string.dump",
        ] {
            assert!(!ALLOWED.contains(&name), "{name} is on the allowlist");
        }
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
