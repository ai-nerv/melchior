//! What one magi calls itself: `project/role/id` — the working directory's name, which a
//! `.magi.lua` may override, what this session is for, and which session it is. The socket
//! address is `<project>/<id>` with no role, because an id is already unique inside a project and
//! a session that changed what it was for would otherwise move. Permission never reads the role,
//! since a session picks its own; what decides reach is the tree — see [`crate::policy`].

const ROLE: &str = "main";

/// What a session calls itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub project: String,
    pub role: String,
    pub id: String,
}

impl Identity {
    /// Work out who this session is, `named` being what a config called the project.
    #[must_use]
    pub fn here(named: Option<&str>) -> Self {
        let project = named
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(folder);
        Self {
            project,
            role: ROLE.to_owned(),
            id: name(),
        }
    }

    /// The whole name, as it is shown and as another session would address it.
    #[must_use]
    pub fn full(&self) -> String {
        format!("{}/{}/{}", self.project, self.role, self.id)
    }

    /// Read one back from a name that came off the wire: three parts, or two with the role left
    /// out, which is what an older session or a hand-written client sends.
    #[must_use]
    pub fn read(whole: &str) -> Option<Self> {
        let parts: Vec<&str> = whole.split('/').filter(|part| !part.is_empty()).collect();
        match parts.as_slice() {
            [project, id] => Some(Self {
                project: (*project).to_owned(),
                role: ROLE.to_owned(),
                id: (*id).to_owned(),
            }),
            [project, role, id] => Some(Self {
                project: (*project).to_owned(),
                role: (*role).to_owned(),
                id: (*id).to_owned(),
            }),
            _ => None,
        }
    }
}

/// The working directory's own name: the last component, not the path, and never empty.
fn folder() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "magi".to_owned())
}

/// The Greek alphabet, which is what an id is drawn from.
const GREEK: [&str; 24] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi",
    "psi", "omega",
];

/// A name for this session, drawn from the clock.
fn name() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize);
    from_seed(seed)
}

/// A name none of `taken` is already listening under. The taken names are a parameter rather
/// than read from the directory, so this module and [`crate::directory::free_in`] do not depend
/// on each other; still a guess, because the look and the bind are not one act.
#[must_use]
pub fn free_of(project: &str, taken: &[String]) -> Identity {
    let from = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize);
    let picked = (0..GREEK.len() * (GREEK.len() - 1))
        .map(|step| from_seed(from.wrapping_add(step)))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| from_seed(from));
    Identity {
        project: project.to_owned(),
        role: ROLE.to_owned(),
        id: picked,
    }
}

/// A secret to hand a session being started, which a `stop` has to quote back: sixteen bytes of
/// `/dev/urandom` as hex, never written to disk or into a transcript.
#[must_use]
pub fn secret() -> String {
    use std::io::Read;
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .expect("the kernel's randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The name a seed picks, always a pair of different words — `delta-rho`, never `delta` and
/// never `rho-rho`.
#[must_use]
fn from_seed(seed: usize) -> String {
    let first = seed % GREEK.len();
    let apart = 1 + (seed / GREEK.len()) % (GREEK.len() - 1);
    format!("{}-{}", GREEK[first], GREEK[(first + apart) % GREEK.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_three_parts() {
        let me = Identity::here(Some("thing"));
        assert_eq!(me.full(), format!("thing/main/{}", me.id));
        assert_eq!(me.full().split('/').count(), 3);
    }

    #[test]
    fn a_name_reads_back_into_the_three_it_was_made_of() {
        let me = Identity::here(Some("thing"));
        assert_eq!(Identity::read(&me.full()).as_ref(), Some(&me));
    }

    #[test]
    fn a_name_with_the_role_left_out_still_reads() {
        let short = Identity::read("magi/delta-rho").expect("a name");
        assert_eq!(short.project, "magi");
        assert_eq!(short.id, "delta-rho");
        assert_eq!(short.role, "main");
    }

    #[test]
    fn what_is_not_a_name_is_not_read_as_one() {
        for written in ["", "magi", "a/b/c/d", "/"] {
            assert_eq!(Identity::read(written), None, "{written:?}");
        }
    }

    #[test]
    fn a_config_may_name_the_project_and_an_empty_name_is_not_a_name() {
        assert_eq!(Identity::here(Some("chosen")).project, "chosen");
        assert_eq!(Identity::here(Some("   ")).project, folder());
        assert_eq!(Identity::here(None).project, folder());
    }

    #[test]
    fn the_project_is_the_folder_rather_than_the_path() {
        assert!(!folder().contains('/'), "{}", folder());
        assert!(!folder().is_empty());
    }

    #[test]
    fn every_id_is_a_pair() {
        for seed in 0..2000 {
            let id = from_seed(seed);
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.len(), 2, "{id} is not a pair");
            assert!(GREEK.contains(&parts[0]), "{id}");
            assert!(GREEK.contains(&parts[1]), "{id}");
        }
        assert_eq!(from_seed(0), "alpha-beta");
    }

    #[test]
    fn the_two_halves_are_never_the_same_word() {
        for seed in 0..2000 {
            let id = from_seed(seed);
            let (first, second) = id.split_once('-').expect("a pair");
            assert_ne!(first, second, "{id}");
        }
    }

    #[test]
    fn there_are_enough_of_them_to_go_round() {
        let mut seen: Vec<String> = (0..600).map(from_seed).collect();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() > 500, "only {} distinct names", seen.len());
    }

    #[test]
    fn a_name_never_contains_the_separator_it_is_joined_with() {
        for seed in 0..1000 {
            assert!(!from_seed(seed).contains('/'), "{}", from_seed(seed));
        }
    }
}
