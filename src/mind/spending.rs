//! The spending cap. Every ask adds what it cost to its run's ledger, which the root session and
//! every session started under it share, and an ask whose run has reached `spend_cap_usd` is
//! refused before anything is sent.

use crate::mind::model::{Cost, Usage};
use std::path::{Path, PathBuf};

/// The setting, as a config file and a coordinator name it.
pub const SETTING: &str = "spend_cap_usd";

/// The cap when nothing sets one, in US dollars.
pub const DEFAULT_USD: f64 = 10.0;

/// How long an ask waits for another to finish adding to the same ledger.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// The cap `said` names, in millionths of a dollar: unset is the default, `0` or `false` is no cap,
/// and anything else that is not a positive number keeps the default.
#[must_use]
pub fn cap(said: Option<&serde_json::Value>) -> Option<u64> {
    let usd = match said {
        Some(serde_json::Value::Bool(false)) => return None,
        Some(serde_json::Value::Number(number)) => {
            number.as_f64().filter(|usd| usd.is_finite() && *usd >= 0.0)
        }
        _ => None,
    }
    .unwrap_or(DEFAULT_USD);
    (usd > 0.0).then(|| micros(usd).max(1))
}

/// What `usage` cost, in millionths of a dollar: what the provider said, else the model's own
/// prices, and nothing for a model that has none.
#[must_use]
pub fn cost(usage: Usage, price: Cost) -> u64 {
    if usage.cost_micros > 0 {
        usage.cost_micros
    } else {
        micros(usage.price(price))
    }
}

fn micros(usd: f64) -> u64 {
    (usd.max(0.0) * 1_000_000.0).round() as u64
}

/// What an ask is told when its run has spent its cap, or `None` while it may still ask.
#[must_use]
pub fn refused(spent: u64, cap: Option<u64>) -> Option<String> {
    let cap = cap?;
    (spent >= cap).then(|| {
        format!(
            "spending cap reached: this run has spent {} of its {} cap, so this request was not \
             sent. To go on, set melchior.spend_cap_usd higher, or to 0 or false for no cap",
            dollars(spent),
            dollars(cap)
        )
    })
}

/// Millionths of a dollar as dollars: to the cent, and further only where it is not round.
fn dollars(micros: u64) -> String {
    let fraction = format!("{:06}", micros % 1_000_000);
    let kept = fraction.trim_end_matches('0');
    let shown = if kept.len() > 2 {
        kept
    } else {
        fraction.get(..2).unwrap_or("00")
    };
    format!("${}.{shown}", micros / 1_000_000)
}

/// Which run this process's asks are charged to: the one it was started under, else the one the
/// session sharing its coordinator's directory noted. `None` for an ask no session started.
#[must_use]
pub fn run() -> Option<String> {
    crate::directory::sessions::inherited()
        .or_else(|| crate::directory::sessions::run_given(&crate::mind::setup::given_dir()?))
}

/// One run's ledger: what every ask in it has cost, in millionths of a dollar.
#[derive(Debug, Clone)]
pub struct Ledger {
    dir: PathBuf,
    name: String,
}

impl Ledger {
    /// The ledger for `run`, kept in `dir`.
    #[must_use]
    pub fn at(dir: &Path, run: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            name: crate::directory::safe(run),
        }
    }

    /// Where melchior keeps every run's ledger, beside the project directories.
    #[must_use]
    pub fn kept() -> PathBuf {
        crate::directory::root().join(".spent")
    }

    /// The ledger of the run this process's asks belong to.
    #[must_use]
    pub fn mine() -> Option<Self> {
        run().map(|run| Self::at(&Self::kept(), &run))
    }

    /// What the run has spent; a ledger never written is nothing spent.
    #[must_use]
    pub fn spent(&self) -> u64 {
        read(&self.beside("")).unwrap_or(0)
    }

    /// Add `micros` under the ledger's lock, written beside and renamed into place, and say the
    /// run's total.
    ///
    /// # Errors
    /// When the directory, the lock or the ledger cannot be had, or another ask holds the lock for
    /// five seconds.
    pub fn add(&self, micros: u64) -> Result<u64, String> {
        let _held = self.lock()?;
        let total = self.spent().saturating_add(micros);
        let (next, ledger) = (self.beside(".next"), self.beside(""));
        private(&next, true)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, format!("{total}\n").as_bytes())
            })
            .and_then(|()| std::fs::rename(&next, &ledger))
            .map_err(|why| format!("{}: {why}", ledger.display()))?;
        Ok(total)
    }

    /// Take the ledger and its lock down, and the directory with them when nothing else is in it.
    pub fn close(&self) {
        for suffix in ["", ".lock", ".next"] {
            let _ = std::fs::remove_file(self.beside(suffix));
        }
        let _ = std::fs::remove_dir(&self.dir);
    }

    fn beside(&self, suffix: &str) -> PathBuf {
        self.dir.join(format!("{}{suffix}", self.name))
    }

    /// The ledger's lock, held until the file is dropped and by nobody once its holder exits.
    fn lock(&self) -> Result<std::fs::File, String> {
        crate::places::ensure(&self.dir)?;
        let at = self.beside(".lock");
        let file = private(&at, false).map_err(|why| format!("{}: {why}", at.display()))?;
        let until = std::time::Instant::now() + PATIENCE;
        loop {
            match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => return Ok(file),
                Err(rustix::io::Errno::WOULDBLOCK) if std::time::Instant::now() < until => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(why) => return Err(format!("{}: {why}", at.display())),
            }
        }
    }
}

/// `path` opened for writing, `0600` when it is made here, and never through a link.
fn private(path: &Path, truncate: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(truncate)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
        .open(path)
}

/// The total a ledger holds, never read through a link.
fn read(path: &Path) -> Option<u64> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut said = String::new();
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
        .open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut said)
        .ok()?;
    said.trim().parse().ok()
}

/// What one ask has added to its run's ledger, so each `Spent` adds only what is new.
#[derive(Debug)]
pub struct Meter {
    ledger: Option<Ledger>,
    price: Cost,
    charged: u64,
}

impl Meter {
    /// An ask that has cost nothing yet, charging `ledger` at `price` where the provider says none.
    #[must_use]
    pub fn new(ledger: Option<Ledger>, price: Cost) -> Self {
        Self {
            ledger,
            price,
            charged: 0,
        }
    }

    /// The whole ask has now cost `usage`, failed attempts included; the ledger is told the rest.
    pub fn heard(&mut self, usage: Usage) {
        let now = cost(usage, self.price);
        if now <= self.charged {
            return;
        }
        let more = now - self.charged;
        self.charged = now;
        if let Some(ledger) = &self.ledger
            && let Err(why) = ledger.add(more)
        {
            crate::noted!("ask: what it cost was not kept: {why}");
        }
    }
}

#[cfg(test)]
#[path = "spending/tests.rs"]
mod tests;
