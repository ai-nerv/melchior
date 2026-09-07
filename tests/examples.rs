//! The shipped examples, run.
//!
//! pi ships roughly seventy-eight example extensions. That is not documentation — it is how they
//! know the extension surface works, and melchior's had never been used by anybody who did not
//! write it. An example that does not load is worse than no example, because somebody copies it.
//!
//! These run each file the way a plugin directory would — in a plain VM, over the shipped
//! catalog — and check that it declared what it says it declares.

use melchior::mind::catalog::Catalog;
use melchior::scratch::Scratch;

/// One example, read from the tree at run time — the way a plugin directory reads one.
fn example(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/plugin")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
}

/// A config directory with the examples installed as plugins.
fn installed(under: &str, names: &[&str]) -> Scratch {
    let dir = Scratch::new("melchior-examples", under);
    std::fs::create_dir_all(dir.join("plugin")).expect("mkdir");
    for name in names {
        std::fs::write(dir.join("plugin").join(name), example(name)).expect("write");
    }
    dir
}

#[test]
fn the_provider_example_adds_endpoints_without_touching_the_shipped_ones() {
    let dir = installed("providers", &["local-llama.lua"]);
    let catalog = Catalog::load(&dir).expect("loads");
    let named: Vec<&str> = catalog.providers.iter().map(|p| p.id.as_str()).collect();

    assert!(named.contains(&"local-llama"), "{named:?}");
    assert!(named.contains(&"local-ollama"), "{named:?}");
    assert!(
        named.contains(&"anthropic"),
        "and the shipped catalog is untouched, which is the whole point: {named:?}"
    );
}

#[test]
fn the_protocol_example_registers_a_dialect_built_out_of_a_shipped_one() {
    // The claim P3 makes, checked: one wire protocol is a file of its own rather than a fork of
    // the eight hundred lines that ship. It borrows three of the four functions from
    // `openai-completions`, which means `melchior.apis` has to be readable — it was write-only.
    let dir = installed("protocols", &["thinking-tags.lua"]);
    let mut catalog = Catalog::load(&dir).expect("loads");
    let known = catalog.engine.apis();

    assert!(
        known.iter().any(|api| api == "openai-completions-thinking"),
        "{known:?}"
    );
    assert!(
        known.iter().any(|api| api == "openai-completions"),
        "and the one it was built from is still itself: {known:?}"
    );
}
