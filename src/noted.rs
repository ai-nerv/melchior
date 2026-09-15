//! A log for a person watching: every line to `$MELCHIOR_DEBUG_LOG`, the family's shared
//! `$NERV_LOG`, or both, stamped with when it happened and which process said it. melchior nulls
//! the stderr of everything it starts, so this is where a diagnosis goes, and only when asked for.

use std::io::Write;

/// This program's own log.
pub const VARIABLE: &str = "MELCHIOR_DEBUG_LOG";

/// The log every program of the family writes to, so one file reads as one timeline.
pub const FAMILY: &str = "NERV_LOG";

const PROGRAM: &str = "melchior";

/// How much of one value a line carries; see [`short`].
const SHORT: usize = 120;

/// Whether anybody asked for a log.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(VARIABLE).is_some() || std::env::var_os(FAMILY).is_some()
}

/// Append one line to every log that is asked for, once to a file both name. Silent when none is,
/// and silent when a file cannot be opened.
pub fn note(args: std::fmt::Arguments<'_>) {
    let own = std::env::var_os(VARIABLE);
    let family = std::env::var_os(FAMILY).filter(|path| Some(path) != own.as_ref());
    if own.is_none() && family.is_none() {
        return;
    }
    let said = line(std::time::SystemTime::now(), std::process::id(), args);
    for path in own.iter().chain(family.iter()) {
        append(std::path::Path::new(path), &said);
    }
}

/// The half that does not read the environment, so a test can exercise it: `set_var` is `unsafe`
/// under this edition and `unsafe` is denied across the workspace.
pub fn note_to(path: &std::path::Path, args: std::fmt::Arguments<'_>) {
    append(
        path,
        &line(std::time::SystemTime::now(), std::process::id(), args),
    );
}

/// One line, as written: `2026-09-15T14:03:07.123Z melchior[4242] area: message`.
#[must_use]
pub fn line(at: std::time::SystemTime, pid: u32, args: std::fmt::Arguments<'_>) -> String {
    let said = args.to_string().replace(['\n', '\r'], " ");
    format!("{} {PROGRAM}[{pid}] {said}", stamp(at))
}

/// A value cut to fit a line: one line, and no longer than [`SHORT`] characters.
#[must_use]
pub fn short(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SHORT) {
        Some((at, _)) => format!("{}…", &flat[..at]),
        None => flat,
    }
}

/// Written whole in one call, so lines from several processes appending at once do not interleave.
fn append(path: &std::path::Path, said: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(format!("{said}\n").as_bytes());
    }
}

/// UTC to the millisecond, as ISO 8601.
fn stamp(at: std::time::SystemTime) -> String {
    let since = at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (year, month, day) = civil(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's civil-from-days.
fn civil(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// Write one line to every asked-for log, formatted like `println!`. A macro rather than a
/// function so the arguments are not evaluated when nobody asked.
#[macro_export]
macro_rules! noted {
    ($($arg:tt)*) => {
        if $crate::noted::enabled() {
            $crate::noted::note(format_args!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::{FAMILY, VARIABLE, line, note_to, short};
    use crate::scratch::Scratch;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn lines_are_appended_rather_than_replacing_each_other() {
        let at = Scratch::file("melchior-noted", "append", "log.txt");
        note_to(&at, format_args!("{} exited {}", "models", 1));
        note_to(&at, format_args!("and again"));
        let held = std::fs::read_to_string(&at).expect("the log");
        let lines: Vec<&str> = held.lines().collect();
        assert_eq!(lines.len(), 2, "{held}");
        assert!(lines[0].ends_with("] models exited 1"), "{held}");
        assert!(lines[1].ends_with("] and again"), "{held}");
    }

    #[test]
    fn a_line_says_when_which_program_and_which_process() {
        let at = UNIX_EPOCH + Duration::from_millis(951_782_400_500);
        assert_eq!(
            line(at, 42, format_args!("ask: {}", "start")),
            "2000-02-29T00:00:00.500Z melchior[42] ask: start"
        );
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_007);
        assert_eq!(
            line(at, 7, format_args!("x")),
            "2023-11-14T22:13:20.007Z melchior[7] x"
        );
        assert_eq!(
            line(UNIX_EPOCH, 1, format_args!("x")),
            "1970-01-01T00:00:00.000Z melchior[1] x"
        );
    }

    #[test]
    fn a_line_is_one_line() {
        let said = line(UNIX_EPOCH, 1, format_args!("a\nb\r\nc"));
        assert!(!said.contains('\n') && !said.contains('\r'), "{said:?}");
    }

    #[test]
    fn a_long_value_is_cut_and_says_so() {
        let long = "word ".repeat(100);
        let cut = short(&long);
        assert!(cut.ends_with('…'), "{cut}");
        assert_eq!(cut.chars().count(), 121);
        assert_eq!(short("a\n  b"), "a b");
    }

    #[test]
    fn a_log_that_cannot_be_opened_is_not_an_error() {
        note_to(
            std::path::Path::new("/proc/nonexistent/nope"),
            format_args!("into the void"),
        );
    }

    #[test]
    fn nothing_is_written_when_nobody_asked() {
        assert!(
            std::env::var_os(VARIABLE).is_none() && std::env::var_os(FAMILY).is_none(),
            "the suite sets no log"
        );
        let at = Scratch::file("melchior-noted", "quiet", "log.txt");
        noted!("nobody asked");
        assert!(!at.exists(), "{}", at.display());
    }
}
