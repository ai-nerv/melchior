//! atom.

/// Returns this crate's display name.
pub fn name() -> &'static str {
    "atom"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_name() {
        assert_eq!(name(), "atom");
    }
}
