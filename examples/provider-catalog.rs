#[path = "../catalog/compiler/mod.rs"]
mod compiler;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(args.len() == 1, "use --check or --write");
    let mode = args.first().map(String::as_str).unwrap_or_default();
    anyhow::ensure!(
        matches!(mode, "--check" | "--write"),
        "use --check or --write"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bytes = compiler::generate(root)?;
    let path = root.join(compiler::OUTPUT);
    if mode == "--check" {
        anyhow::ensure!(
            std::fs::read(&path)? == bytes,
            "generated catalog differs; run oslo make catalog-generate and review changes"
        );
        println!("catalog matches pinned local inputs; provider activation remains unreviewed");
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("catalog directory"))?;
        melchior::places::ensure(parent).map_err(anyhow::Error::msg)?;
        let temporary = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        use std::io::Write;
        let result = (|| -> std::io::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &path)?;
            std::fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
        println!(
            "wrote {}; no runtime provider declarations changed",
            compiler::OUTPUT
        );
    }
    Ok(())
}
