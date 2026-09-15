//! Where declarations come from, and in what order: neovim's model, unchanged — a runtimepath of
//! roots, `plugin/` run at startup, `after/` last.
//!
//! `apis.lua` and `providers.lua` are not part of it; they are compiled in and layered by
//! [`super::catalog`]. A discovered file runs in the same sandboxed VM as the shipped ones, with
//! the removals already done before any of them run.

use std::path::{Path, PathBuf};

/// Where a file came from, which is what decides whether it runs on sight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// The owner's own. Runs on sight.
    Owner,
    /// A package installed under `site/`, which runs once it has been acknowledged and stops
    /// again the moment it changes.
    Installed,
}

impl Trust {
    /// Whether it has to be acknowledged before it runs.
    #[must_use]
    pub fn needs_acknowledging(self) -> bool {
        matches!(self, Self::Installed)
    }
}

/// The roots declarations are read from.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    /// The config directory — `$MELCHIOR_CONFIG`, or `$XDG_CONFIG_HOME/melchior`.
    pub config: Option<PathBuf>,
    /// `$XDG_DATA_HOME/melchior/site`, where installed packages live.
    pub site: Option<PathBuf>,
}

impl Roots {
    /// The usual roots, for a configuration directory the caller has already decided on. There is
    /// no `given` root: what a coordinator says arrives through `configure` as settings.
    #[must_use]
    pub fn at(config: &Path) -> Self {
        Self {
            config: Some(config.to_owned()),
            site: data_home().map(|home| home.join("melchior/site")),
        }
    }
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
}

/// Every file to read, in the order to read it.
///
/// ```text
///   <config>/plugin/*.lua              alphabetical, each on its own
///   <site>/pack/*/start/*/plugin/*.lua installed packages
///   <config>/after/plugin/*.lua        the last word
/// ```
///
/// `apis.lua` and `providers.lua` are not here: they are compiled in and layered by the caller.
#[must_use]
pub fn runtimepath(roots: &Roots) -> Vec<(PathBuf, Trust)> {
    let mut out = Vec::new();

    if let Some(config) = &roots.config {
        out.extend(
            lua_files(&config.join("plugin"))
                .into_iter()
                .map(|path| (path, Trust::Owner)),
        );
    }

    if let Some(site) = &roots.site {
        for package in packages(&site.join("pack")) {
            out.extend(
                lua_files(&package.join("plugin"))
                    .into_iter()
                    .map(|path| (path, Trust::Installed)),
            );
        }
    }

    // `after/` runs last, and the registrar replaces on `(registrar, id)`, so it wins.
    if let Some(config) = &roots.config {
        out.extend(
            lua_files(&config.join("after/plugin"))
                .into_iter()
                .map(|path| (path, Trust::Owner)),
        );
    }
    out
}

/// Every `.lua` directly in a directory, sorted rather than left in the order the filesystem
/// answers, so the load order is the same on every machine.
fn lua_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|end| end == "lua"))
        .collect();
    found.sort();
    found
}

/// Every installed package under `pack/*/start/*`.
fn packages(pack: &Path) -> Vec<PathBuf> {
    let Ok(groups) = std::fs::read_dir(pack) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for group in groups.flatten() {
        let Ok(entries) = std::fs::read_dir(group.path().join("start")) else {
            continue;
        };
        found.extend(
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, "-- nothing\n").expect("write");
    }

    #[test]
    fn dropping_a_file_in_plugin_is_enough_to_declare_a_protocol() {
        let dir = Scratch::new("melchior-rtp", "dropped");
        touch(&dir.join("plugin/mine.lua"));

        let files = runtimepath(&Roots {
            config: Some(dir.to_path_buf()),
            site: None,
        });
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].0.ends_with("plugin/mine.lua"));
        assert!(
            !files[0].1.needs_acknowledging(),
            "your own file runs on sight"
        );
    }

    #[test]
    fn the_order_is_plugin_then_pack_then_after() {
        let dir = Scratch::new("melchior-rtp", "order");
        let config = dir.join("config");
        let site = dir.join("site");
        touch(&config.join("plugin/a.lua"));
        touch(&site.join("pack/vendor/start/thing/plugin/b.lua"));
        touch(&config.join("after/plugin/c.lua"));

        let files = runtimepath(&Roots {
            config: Some(config),
            site: Some(site),
        });
        let names: Vec<_> = files
            .iter()
            .filter_map(|(path, _)| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert_eq!(names, ["a.lua", "b.lua", "c.lua"]);

        let theirs: Vec<_> = files
            .iter()
            .filter(|(_, trust)| trust.needs_acknowledging())
            .filter_map(|(path, _)| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert_eq!(theirs, ["b.lua"]);
    }

    #[test]
    fn nothing_installed_is_no_files_rather_than_an_error() {
        let dir = Scratch::new("melchior-rtp", "empty");
        let files = runtimepath(&Roots {
            config: Some(dir.join("nowhere")),
            site: Some(dir.join("also-nowhere")),
        });
        assert!(files.is_empty(), "{files:?}");
    }

    #[test]
    fn only_lua_files_are_picked_up() {
        let dir = Scratch::new("melchior-rtp", "kinds");
        touch(&dir.join("plugin/real.lua"));
        touch(&dir.join("plugin/README.md"));
        touch(&dir.join("plugin/real.lua.bak"));

        let files = runtimepath(&Roots {
            config: Some(dir.to_path_buf()),
            site: None,
        });
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].0.ends_with("real.lua"));
    }
}
