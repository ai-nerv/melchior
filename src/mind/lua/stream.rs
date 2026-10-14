//! The socket primitive the family's Lua clients need: they carry framing and encoding
//! themselves, but cannot open a socket, so the host lends one as a native.
//!
//! ```lua
//! local h = magi.stream.connect(path, timeout_ms)
//! h:send(bytes)   h:recv(n)   h:close()
//! ```

use luna::{Callback, CallbackReturn, Context, Table, Value};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::Duration;

/// The most a single `recv` will be asked for, bounding what a peer claiming an enormous frame
/// can make us allocate before a byte of it has arrived.
const MAX_RECV: usize = 16 * 1024 * 1024;

/// A connected socket, shared between the handle's methods.
type Handle = Rc<RefCell<Option<UnixStream>>>;

/// The socket a config may dial at `path`, resolved: one of the family's own, in its runtime
/// directory, and nothing else there — not the session bus, the agents or systemd beside them.
fn dialable(path: &std::path::Path) -> Option<std::path::PathBuf> {
    crate::places::family_socket(path, &crate::places::family_roots(), crate::places::uid())
}

pub fn table<'gc>(ctx: Context<'gc>) -> Table<'gc> {
    let stream = Table::new(&ctx);
    let connect = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let (path, timeout_ms): (Value, Value) = stack.consume(ctx)?;
        let Value::String(path) = path else {
            stack.replace(ctx, (Value::Nil, "connect needs a path"));
            return Ok(CallbackReturn::Return);
        };
        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();

        // Refused as `nil` and a reason rather than a raise, so a config probing for an absent
        // sibling carries on without it.
        let Some(path) = dialable(std::path::Path::new(&path)) else {
            stack.replace(
                ctx,
                (
                    Value::Nil,
                    "only a family program's own socket is this VM's to open",
                ),
            );
            return Ok(CallbackReturn::Return);
        };

        // A default rather than a wait forever: a stale socket left by a killed peer accepts and
        // never answers.
        let timeout = match timeout_ms {
            Value::Integer(ms) if ms > 0 => Duration::from_millis(ms as u64),
            Value::Number(ms) if ms > 0.0 => Duration::from_millis(ms as u64),
            _ => Duration::from_secs(5),
        };

        match UnixStream::connect(&path) {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(timeout));
                let _ = socket.set_write_timeout(Some(timeout));
                let handle = handle_table(ctx, Rc::new(RefCell::new(Some(socket))));
                stack.replace(ctx, handle);
            }
            Err(e) => {
                stack.replace(ctx, (Value::Nil, e.to_string()));
            }
        }
        Ok(CallbackReturn::Return)
    });
    stream.set(ctx, "connect", connect).ok();
    stream
}

/// A handle, as the client expects: `send`, `recv`, `close`, called with `:`.
fn handle_table<'gc>(ctx: Context<'gc>, socket: Handle) -> Table<'gc> {
    let handle = Table::new(&ctx);

    let held = Rc::clone(&socket);
    let send = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // Called as `h:send(bytes)`, so the handle itself is the first argument.
        let (_self, bytes): (Value, Value) = stack.consume(ctx)?;
        let Value::String(bytes) = bytes else {
            stack.replace(ctx, (Value::Nil, "send needs a string"));
            return Ok(CallbackReturn::Return);
        };
        let mut slot = held.borrow_mut();
        let Some(socket) = slot.as_mut() else {
            stack.replace(ctx, (Value::Nil, "the connection is closed"));
            return Ok(CallbackReturn::Return);
        };
        match socket
            .write_all(bytes.as_bytes())
            .and_then(|()| socket.flush())
        {
            Ok(()) => stack.replace(ctx, true),
            Err(e) => stack.replace(ctx, (Value::Nil, e.to_string())),
        }
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "send", send).ok();

    let held = Rc::clone(&socket);
    let recv = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let (_self, want): (Value, Value) = stack.consume(ctx)?;
        let want = match want {
            Value::Integer(n) if n > 0 => (n as usize).min(MAX_RECV),
            Value::Number(n) if n > 0.0 => (n as usize).min(MAX_RECV),
            _ => 0,
        };
        let mut slot = held.borrow_mut();
        let Some(socket) = slot.as_mut() else {
            stack.replace(ctx, (Value::Nil, "the connection is closed"));
            return Ok(CallbackReturn::Return);
        };
        let mut buffer = vec![0_u8; want];
        match socket.read(&mut buffer) {
            // A short read is ordinary and the client asks again; zero means the peer hung up.
            Ok(read) => {
                buffer.truncate(read);
                let text = luna::String::from_slice(&ctx, &buffer);
                stack.replace(ctx, text);
            }
            Err(e) => stack.replace(ctx, (Value::Nil, e.to_string())),
        }
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "recv", recv).ok();

    let held = Rc::clone(&socket);
    let close = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // Taking the stream closes it and makes a second close a no-op, which client cleanup
        // paths rely on.
        held.borrow_mut().take();
        stack.replace(ctx, true);
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "close", close).ok();

    handle
}

/// A config reaches the family's sockets and nothing else that listens beside them.
#[cfg(test)]
mod tests {
    use crate::mind::lua::engine::Engine;
    use crate::scratch::Scratch;

    #[test]
    fn a_socket_outside_the_family_is_refused_by_the_vm() {
        let dir = Scratch::new("melchior-stream", "bus");
        let bus = dir.join("bus");
        let _held = std::os::unix::net::UnixListener::bind(&bus).expect("bound");
        let mut engine = Engine::new();
        engine
            .run(
                &format!(
                    "local h, why = melchior.stream.connect({:?}, 500)\n\
                     melchior.opened = h ~= nil\n\
                     melchior.why = tostring(why)",
                    bus.display().to_string()
                ),
                "dial.lua",
            )
            .expect("runs");
        engine.harvest();
        assert_eq!(engine.config().boolean("opened"), Some(false));
        let why = engine.config().string("why").unwrap_or_default().to_owned();
        assert!(why.contains("family"), "{why}");
    }
}
