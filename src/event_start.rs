use melchior::events::forwarding::{Forwarder, receive_stdin};

pub const CAPABILITIES: &[u8] = b"{\"ok\":true,\"family\":1,\"n\":1,\"result\":[{\"schema\":1,\"kind\":\"native_event_capabilities\",\"program\":\"melchior\",\"bootstrap\":2}]}\n";

pub fn requested(args: &mut Vec<String>) -> std::io::Result<bool> {
    let count = args
        .iter()
        .filter(|arg| *arg == "--event-bootstrap")
        .count();
    if count == 0 {
        return Ok(false);
    }
    if count != 1 || !matches!(args.first().map(String::as_str), Some("ask" | "serve")) {
        return Err(std::io::Error::other(
            "native event bootstrap requires one flag on ask or serve",
        ));
    }
    args.retain(|arg| arg != "--event-bootstrap");
    Ok(true)
}

pub fn activate(requested: bool) -> std::io::Result<Option<Forwarder>> {
    if !requested {
        return Ok(None);
    }
    let sender =
        receive_stdin().map_err(|_| std::io::Error::other("invalid native event bootstrap"))?;
    Ok(Forwarder::activate(sender).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_activation_is_explicit_and_preserves_other_flags() {
        for verb in ["ask", "serve"] {
            let mut args = vec![verb.into(), "--event-bootstrap".into(), "--json".into()];
            assert!(requested(&mut args).is_ok_and(|enabled| enabled));
            assert_eq!(args, [verb, "--json"]);
        }
        for args in [
            vec!["tool", "--event-bootstrap"],
            vec!["--event-bootstrap", "ask"],
            vec!["serve", "--event-bootstrap", "--event-bootstrap"],
        ] {
            assert!(requested(&mut args.into_iter().map(String::from).collect()).is_err());
        }
        assert!(requested(&mut vec!["ask".into()]).is_ok_and(|enabled| !enabled));
        assert!(activate(false).is_ok_and(|worker| worker.is_none()));
    }
}
