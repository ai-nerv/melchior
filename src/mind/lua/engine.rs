//! The VM, and the config API it hands `init.lua`.

use crate::mind::lua::LuaError;
use crate::mind::lua::convert::json_from_lua;
use luna::{Callback, CallbackReturn, Closure, Executor, Lua, Table, Value};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// What a config declared, once every file has run.
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Settings assigned onto the module, as JSON.
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// Everything handed to a registrar, keyed by registrar then by identity.
    pub registered: Registered,
    /// Files `magi.load` asked for, in the order it asked. Queued rather than run on the spot,
    /// since the VM offers no re-entrancy, and never queued twice, so a diamond terminates.
    pub loads: Vec<String>,
}

impl Config {
    /// A setting, if the config assigned one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.settings.get(name)
    }

    /// A setting as a string.
    #[must_use]
    pub fn string(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(serde_json::Value::as_str)
    }

    /// A setting as a boolean.
    #[must_use]
    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(serde_json::Value::as_bool)
    }

    /// A setting as a number. Lua has one number type, so `2` and `2.0` both answer here.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(serde_json::Value::as_f64)
    }

    /// Everything handed to one registrar, in declaration order.
    #[must_use]
    pub fn all(&self, registrar: &str) -> Vec<(&str, &serde_json::Value)> {
        self.registered
            .order
            .iter()
            .filter(|(kind, _)| kind == registrar)
            .filter_map(|(kind, id)| {
                self.registered
                    .entries
                    .get(&(kind.clone(), id.clone()))
                    .map(|value| (id.as_str(), value))
            })
            .collect()
    }
}

/// Declarations handed to registrars, keyed by `(registrar, identity)` so that re-registering
/// replaces rather than appends and a re-read config is idempotent.
#[derive(Debug, Default, Clone)]
pub struct Registered {
    entries: std::collections::HashMap<(String, String), serde_json::Value>,
    /// Declaration order, so a model picker's list does not reshuffle between runs.
    order: Vec<(String, String)>,
}

impl Registered {
    fn insert(&mut self, registrar: &str, id: &str, value: serde_json::Value) {
        let key = (registrar.to_owned(), id.to_owned());
        if !self.entries.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.entries.insert(key, value);
    }
}

/// The Lua VM, holding whatever the config has declared so far.
pub struct Engine {
    lua: Lua,
    config: Rc<RefCell<Config>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// A VM with the `magi` module installed and nothing declared.
    #[must_use]
    pub fn new() -> Self {
        let mut engine = Self {
            lua: Lua::full(),
            config: Rc::new(RefCell::new(Config::default())),
        };
        engine.lua.set_memory_limit(Some(MEMORY));
        engine.install();
        // After install, so a removal cannot be undone by something the installer adds.
        crate::mind::lua::sandbox::apply(&mut engine.lua);
        engine
    }

    /// What the config has declared.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config.borrow().clone()
    }

    /// Run one config file. A load-time raise is fatal and names the file.
    pub fn run_file(&mut self, path: &Path) -> Result<(), LuaError> {
        let source = std::fs::read_to_string(path).map_err(|source| LuaError::Io {
            file: path.display().to_string(),
            source,
        })?;
        self.run(&source, &path.display().to_string())
    }

    /// Run one config chunk, stopped and reported as failing past [`BUDGET`] instructions.
    pub fn run(&mut self, source: &str, chunk: &str) -> Result<(), LuaError> {
        self.run_within(source, chunk, BUDGET)
    }

    /// The same, under a budget handed in.
    pub fn run_within(&mut self, source: &str, chunk: &str, budget: u64) -> Result<(), LuaError> {
        let failed = |message: String| LuaError::Runtime {
            file: chunk.to_owned(),
            message,
        };
        if crate::mind::lua::sandbox::precompiled(source.as_bytes()) {
            return Err(LuaError::Syntax {
                file: chunk.to_owned(),
                message: crate::mind::lua::sandbox::REFUSED_PRECOMPILED.to_owned(),
            });
        }
        let executor = self
            .lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, Some(chunk), source.as_bytes())?;
                Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
            })
            .map_err(|e| LuaError::Syntax {
                file: chunk.to_owned(),
                message: e.to_string(),
            })?;

        let mut spent: u64 = 0;
        loop {
            let mut fuel = luna::Fuel::with(SLICE);
            let finished = self
                .lua
                .enter(|ctx| ctx.fetch(&executor).step(ctx, &mut fuel))
                .map_err(|e| failed(e.to_string()))?;
            if let Some(limit) = self.over_memory() {
                self.lua.enter(|ctx| ctx.fetch(&executor).stop(&ctx));
                return Err(failed(format!("stopped holding more than {limit} bytes")));
            }
            if finished {
                break;
            }
            spent = spent.saturating_add(SLICE.unsigned_abs().into());
            let waiting = self.lua.enter(|ctx| ctx.fetch(&executor).is_waiting());
            if waiting || spent >= budget {
                self.lua.enter(|ctx| ctx.fetch(&executor).stop(&ctx));
                return Err(failed(if waiting {
                    "it waited on something this VM cannot wait for".to_owned()
                } else {
                    format!("stopped after {budget} instructions without finishing")
                }));
            }
        }
        self.lua
            .try_enter(|ctx| ctx.fetch(&executor).take_result::<()>(ctx)?)
            .map_err(|e| failed(e.to_string()))
    }

    /// The ceiling, if the VM still holds more than it once two full collections have run.
    fn over_memory(&mut self) -> Option<usize> {
        let limit = self.lua.memory_limit()?;
        if self.lua.total_memory() > limit {
            self.lua.gc_collect();
            self.lua.gc_collect();
        }
        (self.lua.total_memory() > limit).then_some(limit)
    }

    /// Install the `magi` global and its registrars. Settings are plain fields on the table,
    /// harvested after every file has run rather than intercepted as they are written.
    fn install(&mut self) {
        let config = Rc::clone(&self.config);
        self.lua.enter(|ctx| {
            let melchior = Table::new(&ctx);

            for registrar in REGISTRARS {
                let held = Rc::clone(&config);
                let name = *registrar;
                let callback = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let (id, spec): (Value, Value) = stack.consume(ctx)?;
                    let Value::String(id) = id else {
                        return Err(raise(
                            ctx,
                            &format!("melchior.{name}: the first argument must be a name"),
                        ));
                    };
                    let id = String::from_utf8_lossy(id.as_bytes()).into_owned();

                    let value = json_from_lua(ctx, spec).map_err(|why| {
                        raise(
                            ctx,
                            &format!(
                                "melchior.{name}({id}): this table cannot be described: {why}"
                            ),
                        )
                    })?;
                    held.borrow_mut().registered.insert(name, &id, value);
                    stack.replace(ctx, ());
                    Ok(CallbackReturn::Return)
                });
                melchior.set(ctx, *registrar, callback).ok();
            }

            // The one way a config reaches another file: `init.lua` is the entry point, and what
            // it does not name does not run.
            {
                let held = Rc::clone(&config);
                let load = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let path: Value = stack.consume(ctx)?;
                    let Value::String(path) = path else {
                        return Err(raise(ctx, "magi.load: expects a path"));
                    };
                    let path = String::from_utf8_lossy(path.as_bytes()).into_owned();
                    let mut held = held.borrow_mut();
                    if !held.loads.contains(&path) {
                        held.loads.push(path);
                    }
                    stack.replace(ctx, ());
                    Ok(CallbackReturn::Return)
                });
                melchior.set(ctx, "load", load).ok();
            }

            // A plain settings table made here, so `magi.ui.accent = 1` needs no `magi.ui = {}`
            // first.
            melchior.set(ctx, "ui", Table::new(&ctx)).ok();
            // The socket primitive, named twice: `melchior.stream` for a client that knows this
            // host, `__stream` for one that does not.
            let stream = crate::mind::lua::stream::table(ctx);
            melchior.set(ctx, "stream", stream).ok();
            ctx.set_global("__stream", stream);
            let fs = crate::mind::lua::fs::table(ctx);
            melchior.set(ctx, "fs", fs).ok();
            let json = crate::mind::lua::json::table(ctx);
            melchior.set(ctx, "json", json).ok();

            // Protocols carry functions, which cannot be described as data, so the VM keeps them
            // and Rust keeps only their names.
            let apis = Table::new(&ctx);
            ctx.set_global(APIS, apis);
            // The same table lent back, so a protocol can be built out of one that already
            // exists and registering through either reaches the other.
            melchior.set(ctx, "apis", apis).ok();
            let api = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                let (name, spec): (Value, Value) = stack.consume(ctx)?;
                let (Value::String(name), Value::Table(_)) = (name, spec) else {
                    return Err(raise(ctx, "melchior.api(name, spec): a name and a table"));
                };
                if let Value::Table(apis) = ctx.get_global_value(APIS) {
                    apis.set(ctx, name, spec).ok();
                }
                stack.replace(ctx, ());
                Ok(CallbackReturn::Return)
            });
            melchior.set(ctx, "api", api).ok();

            // The path of the running binary. It is multi-call, so its peers are this same
            // executable under another name, and `command = "magi"` would find whatever is on
            // PATH instead.
            if let Ok(exe) = std::env::current_exe() {
                let path = luna::String::from_slice(&ctx, exe.as_os_str().as_encoded_bytes());
                melchior.set(ctx, "self", path).ok();
            }

            ctx.set_global("melchior", melchior);
        });
    }

    /// Read the settings the config assigned, and forget the module. Called once after every
    /// file has run.
    pub fn harvest(&mut self) {
        let config = Rc::clone(&self.config);
        self.lua.enter(|ctx| {
            let Value::Table(melchior) = ctx.get_global_value("melchior") else {
                return;
            };
            let mut held = config.borrow_mut();
            for (key, value) in melchior.iter(ctx) {
                let Value::String(name) = key else { continue };
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                // A registrar is a function and cannot be described, so it is skipped and every
                // other field is a setting.
                if let Ok(json) = json_from_lua(ctx, value) {
                    held.settings.insert(name, json);
                }
            }
        });
    }
}

/// Roughly how many VM instructions one chunk or one protocol call may run.
pub const BUDGET: u64 = 200_000_000;

/// How many instructions one step of the executor is given.
const SLICE: i32 = 4096;

/// The most memory one VM may hold between slices, in bytes.
pub const MEMORY: usize = 1 << 30;

/// The registrars a config may call; adding one here is the only way a config gains a new kind
/// of declaration.
const REGISTRARS: &[&str] = &["provider"];

/// Raise a message into Lua, so `pcall` in a config sees a string.
fn raise<'gc>(ctx: luna::Context<'gc>, message: &str) -> luna::Error<'gc> {
    luna::Error::from_value(Value::String(luna::String::from_slice(
        &ctx,
        message.as_bytes(),
    )))
}

/// Where registered protocol descriptions live inside the VM, as a Lua table because what is
/// registered is functions, which cannot cross the boundary.
const APIS: &str = "__melchior_apis";

impl Engine {
    /// The protocols a config registered.
    #[must_use]
    pub fn apis(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.lua.enter(|ctx| {
            if let Value::Table(apis) = ctx.get_global_value(APIS) {
                for (key, _) in apis.iter(ctx) {
                    if let Value::String(name) = key {
                        out.push(String::from_utf8_lossy(name.as_bytes()).into_owned());
                    }
                }
            }
        });
        out.sort();
        out
    }

    /// Call one function of a registered protocol, in and out as JSON so the collector lifetime
    /// never leaves this crate. `None` covers a missing protocol, a missing function and a call
    /// that produced nothing alike.
    pub fn call_api(
        &mut self,
        api: &str,
        method: &str,
        args: &[serde_json::Value],
    ) -> Option<serde_json::Value> {
        let args = serde_json::Value::Array(args.to_vec());
        self.lua.enter(|ctx| {
            let value = crate::mind::lua::convert::lua_from_json(ctx, &args);
            ctx.set_global("__melchior_args", value);
        });

        let source = format!(
            "local api = {APIS} and {APIS}[{api:?}]\n\
             local fn = api and api[{method:?}]\n\
             if fn then __melchior_result = fn(table.unpack(__melchior_args)) \
             else __melchior_result = nil end"
        );
        self.run(&source, "api.lua").ok()?;

        let mut out = None;
        self.lua.enter(|ctx| {
            out = json_from_lua(ctx, ctx.get_global_value("__melchior_result")).ok();
        });
        out.filter(|value| !value.is_null())
    }
}

/// A chunk that never finishes is stopped and named, rather than holding whatever ran it.
#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn a_loop_is_stopped_and_reported_as_failing() {
        let mut engine = Engine::new();
        let why = engine
            .run_within("while true do end", "plugin/loop.lua", 1_000_000)
            .expect_err("a loop finished");
        let said = why.to_string();
        assert!(said.contains("plugin/loop.lua"), "{said}");
        assert!(said.contains("1000000 instructions"), "{said}");
    }

    #[test]
    fn the_default_budget_ends_a_loop_too() {
        let mut engine = Engine::new();
        let began = std::time::Instant::now();
        assert!(engine.run("while true do end", "loop.lua").is_err());
        assert!(
            began.elapsed() < std::time::Duration::from_secs(60),
            "{:?}",
            began.elapsed()
        );
    }

    #[test]
    fn work_inside_the_budget_still_finishes_and_the_vm_is_usable_after_a_stop() {
        let mut engine = Engine::new();
        let _ = engine.run_within("while true do end", "loop.lua", 100_000);
        engine
            .run(
                "local n = 0 for i = 1, 100000 do n = n + i end melchior.n = n",
                "sum.lua",
            )
            .expect("an ordinary chunk runs");
        engine.harvest();
        assert_eq!(engine.config().number("n"), Some(5_000_050_000.0));
    }
}

/// A chunk that holds too much is stopped and named, rather than taking the host down with it.
#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn the_shipped_declarations_and_plugins_fit_far_under_the_ceiling() {
        let mut engine = Engine::new();
        for (name, source) in [
            ("apis.lua", include_str!("../../../config/apis.lua")),
            (
                "providers.lua",
                include_str!("../../../config/providers.lua"),
            ),
            (
                "local-llama.lua",
                include_str!("../../../examples/plugin/local-llama.lua"),
            ),
            (
                "thinking-tags.lua",
                include_str!("../../../examples/plugin/thinking-tags.lua"),
            ),
            ("client.lua", crate::CLIENT),
        ] {
            engine.run(source, name).expect(name);
        }
        engine.lua.gc_collect();
        let held = engine.lua.total_memory();
        eprintln!("shipped declarations and plugins hold {held} bytes");
        assert_eq!(engine.lua.memory_limit(), Some(MEMORY));
        assert!(held < MEMORY / 256, "{held} bytes");
    }

    #[test]
    fn a_chunk_over_the_ceiling_is_stopped_and_the_vm_recovers() {
        let mut engine = Engine::new();
        engine.lua.set_memory_limit(Some(8 << 20));
        let why = engine
            .run(
                "local t = {} for i = 1, 1e7 do t[i] = ('x'):rep(64) .. i end",
                "plugin/hog.lua",
            )
            .expect_err("a hog finished");
        let said = why.to_string();
        assert!(said.contains("plugin/hog.lua"), "{said}");
        assert!(said.contains(&(8 << 20).to_string()), "{said}");
        engine
            .run("melchior.after = 1", "after.lua")
            .expect("the VM runs again once the hog is gone");
    }

    #[test]
    fn garbage_is_collected_before_the_ceiling_is_judged() {
        let mut engine = Engine::new();
        engine.lua.set_memory_limit(Some(8 << 20));
        engine
            .run(
                "collectgarbage('stop') for i = 1, 50000 do local s = ('x'):rep(1024) .. i end",
                "churn.lua",
            )
            .expect("garbage alone is not held");
    }
}

/// Bytecode is refused wherever a chunk enters the VM, whatever mode it asks for.
#[cfg(test)]
mod precompiled_tests {
    use super::*;

    /// A real luna chunk that sets `melchior.dumped`, as bytes.
    fn dumped(engine: &mut Engine) -> Vec<u8> {
        engine.lua.enter(|ctx| {
            let closure =
                Closure::load(ctx, Some("dumped"), b"melchior.dumped = true").expect("compiles");
            luna::dump::dump(&closure.prototype(), true)
        })
    }

    #[test]
    fn a_precompiled_file_is_refused_before_it_is_loaded() {
        let mut engine = Engine::new();
        let bytes = dumped(&mut engine);
        let luna = std::str::from_utf8(&bytes).expect("this chunk dumps as UTF-8");
        for source in [luna, "\x1bLuaT\0\x19\u{93}\r\n\x1a\n"] {
            let why = engine
                .run(source, "plugin/dumped.lua")
                .expect_err("bytecode ran");
            let said = why.to_string();
            assert!(said.contains("plugin/dumped.lua"), "{said}");
            assert!(said.contains("precompiled"), "{said}");
        }
        engine.harvest();
        assert_eq!(engine.config().boolean("dumped"), None);
    }

    #[test]
    fn load_refuses_bytecode_in_every_mode_and_still_takes_source() {
        let mut engine = Engine::new();
        let bytes = dumped(&mut engine);
        engine.lua.enter(|ctx| {
            ctx.set_global("chunk", luna::String::from_slice(&ctx, &bytes));
        });
        engine
            .run(
                "local said = {}\n\
                 for _, mode in ipairs({ 'b', 'bt', 't' }) do\n\
                   local ok, why = pcall(load, chunk, 'dumped', mode)\n\
                   said[#said + 1] = tostring(ok) .. ':' .. tostring(why)\n\
                 end\n\
                 melchior.said = table.concat(said, '|')\n\
                 melchior.source = load('return 7')()",
                "loading.lua",
            )
            .expect("runs");
        engine.harvest();
        let said = engine
            .config()
            .string("said")
            .unwrap_or_default()
            .to_owned();
        assert_eq!(
            said.matches("false:load: a precompiled chunk").count(),
            3,
            "{said}"
        );
        assert_eq!(engine.config().boolean("dumped"), None);
        assert_eq!(engine.config().number("source"), Some(7.0));
    }
}

/// Nothing a chunk does queues an object for a finalizer, so nothing waits on one forever.
#[cfg(test)]
mod finalizer_tests {
    use super::*;

    #[test]
    fn a_metatable_with_gc_is_refused_and_any_other_is_taken() {
        let mut engine = Engine::new();
        engine
            .run(
                "local ok, why = pcall(setmetatable, {}, { __gc = function() end })\n\
                 melchior.ok, melchior.why = ok, why\n\
                 melchior.n = setmetatable({}, { __index = { n = 3 } }).n\n\
                 melchior.no_debug = debug == nil",
                "gc.lua",
            )
            .expect("runs");
        engine.harvest();
        let config = engine.config();
        assert_eq!(config.boolean("ok"), Some(false));
        assert!(config.string("why").unwrap_or_default().contains("__gc"));
        assert_eq!(config.number("n"), Some(3.0));
        assert_eq!(config.boolean("no_debug"), Some(true));
    }

    #[test]
    fn dropped_tables_are_freed_rather_than_held_for_a_finalizer() {
        let mut engine = Engine::new();
        engine.lua.gc_collect();
        let before = engine.lua.total_memory();
        engine
            .run(
                "local function gc() end\n\
                 for i = 1, 100000 do\n\
                   pcall(setmetatable, { ('x'):rep(64) .. i }, { __gc = gc })\n\
                   local late = {}\n\
                   setmetatable({ ('y'):rep(64) .. i }, late)\n\
                   late.__gc = gc\n\
                 end",
                "queue.lua",
            )
            .expect("runs");
        engine.lua.gc_collect();
        engine.lua.gc_collect();
        let after = engine.lua.total_memory();
        assert!(after < before + (1 << 20), "{before} -> {after} bytes");
    }
}
