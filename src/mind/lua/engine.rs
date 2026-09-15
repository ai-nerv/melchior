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

    /// Run one config chunk.
    pub fn run(&mut self, source: &str, chunk: &str) -> Result<(), LuaError> {
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

        self.lua
            .execute::<()>(&executor)
            .map_err(|e| LuaError::Runtime {
                file: chunk.to_owned(),
                message: e.to_string(),
            })
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

                    let Some(value) = json_from_lua(ctx, spec, 0) else {
                        return Err(raise(
                            ctx,
                            &format!("melchior.{name}({id}): this table cannot be described"),
                        ));
                    };
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
                if let Some(json) = json_from_lua(ctx, value, 0) {
                    held.settings.insert(name, json);
                }
            }
        });
    }
}

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
            out = crate::mind::lua::convert::json_from_lua(
                ctx,
                ctx.get_global_value("__melchior_result"),
                0,
            );
        });
        out.filter(|value| !value.is_null())
    }
}
