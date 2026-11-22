use sha2::{Digest, Sha256};

pub(crate) fn digest(purpose: &str, fields: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for field in std::iter::once(purpose.as_bytes()).chain(fields.iter().copied()) {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn nonce() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(digest("oauth-login-v1", &[&bytes]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_digests_separate_purposes_and_field_boundaries() {
        assert_ne!(digest("a", &[b"bc"]), digest("ab", &[b"c"]));
        assert_ne!(digest("p", &[b"a", b"bc"]), digest("p", &[b"ab", b"c"]));
        assert_ne!(
            digest("api-key-v1", &[b"secret"]),
            digest("oauth-legacy-v1", &[b"secret"])
        );
        assert_eq!(digest("p", &[b"secret"]), digest("p", &[b"secret"]));
    }
}
