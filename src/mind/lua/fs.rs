//! The directory lister the family's clients use to find each other's sockets. Listing is all
//! that is lent: a `fs.dir` naming the host's own runtime directory would be believed, sending
//! another tool's client looking for its sockets in melchior's directory.

use luna::{Callback, CallbackReturn, Context, Table, Value};

pub fn table<'gc>(ctx: Context<'gc>) -> Table<'gc> {
    let fs = Table::new(&ctx);
    let ls = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let path: Value = stack.consume(ctx)?;
        let Value::String(path) = path else {
            stack.replace(ctx, Value::Nil);
            return Ok(CallbackReturn::Return);
        };
        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();

        let out = Table::new(&ctx);
        // An unreadable directory lists as empty rather than raising, so a client may probe
        // candidates that do not exist.
        if let Ok(entries) = std::fs::read_dir(&path) {
            let mut index = 1_i64;
            for entry in entries.flatten() {
                let record = Table::new(&ctx);
                let name = entry.file_name().to_string_lossy().into_owned();
                record
                    .set(ctx, "name", luna::String::from_slice(&ctx, name.as_bytes()))
                    .ok();

                // Modification time, which clients sort by; absent rather than zero when unknown,
                // since zero sorts as the oldest.
                if let Some(mtime) = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    record.set(ctx, "mtime", mtime.as_secs() as i64).ok();
                }
                out.set(ctx, index, record).ok();
                index += 1;
            }
        }
        stack.replace(ctx, out);
        Ok(CallbackReturn::Return)
    });
    fs.set(ctx, "ls", ls).ok();
    fs
}
