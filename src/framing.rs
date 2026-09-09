//! The family's framing: a big-endian `u32` of length, then the body, with nothing wrapped
//! around it — not the `magi_ipc` envelope the UI and the daemon speak to each other. Anything
//! may knock on this socket, so it speaks what [`crate::wire`] documents. JSON by default and
//! CBOR when that is what turned up: a reply goes back in whichever encoding the call arrived
//! in, decided from the body's first byte rather than negotiated. Both a blocking and an
//! asynchronous half, since the listener lives in a UI that must not block and the caller is a
//! peer whose whole job is one round trip.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{Read, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The largest frame either end will read or write: the socket is reachable by anything running
/// as this user, and an unbounded read is a way to make a session allocate until it dies.
pub const MOST: usize = 1 << 20;

/// Which encoding a body is in. Nothing is negotiated: a body says which it is in its first
/// byte, since JSON's top level here is an object or an array and so begins `{` or `[`, while
/// CBOR's is a map or an array, whose first byte is `0x80`–`0xBF`. The ranges do not overlap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Wire {
    /// Text. The default, and what every peer understands.
    #[default]
    Json,
    Cbor,
}

impl Wire {
    /// Which encoding `body` is in.
    #[must_use]
    pub fn of(body: &[u8]) -> Self {
        match body.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(0x80..=0xBF) => Self::Cbor,
            _ => Self::Json,
        }
    }

    /// Read `body` as `T`, in whichever encoding it is.
    pub fn read_any<T: DeserializeOwned>(body: &[u8]) -> std::io::Result<T> {
        match Self::of(body) {
            Self::Json => serde_json::from_slice(body).map_err(std::io::Error::other),
            Self::Cbor => ciborium::from_reader(body).map_err(std::io::Error::other),
        }
    }

    /// Encode `value` in this encoding.
    pub fn encode<T: Serialize>(self, value: &T) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Json => serde_json::to_vec(value).map_err(std::io::Error::other),
            Self::Cbor => {
                let mut bytes = Vec::new();
                ciborium::into_writer(value, &mut bytes).map_err(std::io::Error::other)?;
                Ok(bytes)
            }
        }
    }
}

/// Encode one value in `how`, framed.
fn framed<T: Serialize>(how: Wire, value: &T) -> std::io::Result<Vec<u8>> {
    let body = how.encode(value)?;
    if body.len() > MOST {
        return Err(std::io::Error::other("that is too much to say at once"));
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// How long a frame says it is, or an error saying why it cannot be read.
fn expecting(header: [u8; 4]) -> std::io::Result<usize> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MOST {
        return Err(std::io::Error::other(format!(
            "a frame of {len} bytes is beyond what this socket reads"
        )));
    }
    Ok(len)
}

/// Read one message from an async stream.
pub async fn read<T: DeserializeOwned, R: AsyncRead + Unpin>(from: &mut R) -> std::io::Result<T> {
    read_wire(from).await.map(|(value, _)| value)
}

/// The same, saying which encoding it arrived in, which is what a server needs to answer in kind.
pub async fn read_wire<T: DeserializeOwned, R: AsyncRead + Unpin>(
    from: &mut R,
) -> std::io::Result<(T, Wire)> {
    let mut header = [0_u8; 4];
    from.read_exact(&mut header).await?;
    let mut body = vec![0_u8; expecting(header)?];
    from.read_exact(&mut body).await?;
    Ok((Wire::read_any(&body)?, Wire::of(&body)))
}

/// Write one message to an async stream.
pub async fn write<T: Serialize, W: AsyncWrite + Unpin>(
    to: &mut W,
    value: &T,
) -> std::io::Result<()> {
    write_as(to, Wire::Json, value).await
}

/// Write one message in `how`.
pub async fn write_as<T: Serialize, W: AsyncWrite + Unpin>(
    to: &mut W,
    how: Wire,
    value: &T,
) -> std::io::Result<()> {
    to.write_all(&framed(how, value)?).await?;
    to.flush().await
}

/// Read one message from a blocking stream.
pub fn read_from<T: DeserializeOwned, R: Read>(from: &mut R) -> std::io::Result<T> {
    let mut header = [0_u8; 4];
    from.read_exact(&mut header)?;
    let mut body = vec![0_u8; expecting(header)?];
    from.read_exact(&mut body)?;
    Wire::read_any(&body)
}

/// Write one message to a blocking stream.
pub fn write_to<T: Serialize, W: Write>(to: &mut W, value: &T) -> std::io::Result<()> {
    write_to_as(to, Wire::Json, value)
}

/// Write one message in `how`, to a blocking stream.
pub fn write_to_as<T: Serialize, W: Write>(
    to: &mut W,
    how: Wire,
    value: &T,
) -> std::io::Result<()> {
    to.write_all(&framed(how, value)?)?;
    to.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Call, Reply};

    #[test]
    fn a_frame_is_four_bytes_of_length_and_then_json() {
        let out = framed(
            Wire::Json,
            &Call {
                call: "status".to_owned(),
                ..Call::default()
            },
        )
        .expect("frames");
        let len = u32::from_be_bytes([out[0], out[1], out[2], out[3]]) as usize;
        assert_eq!(len, out.len() - 4, "the length does not describe the body");
        let body = std::str::from_utf8(&out[4..]).expect("it is text");
        assert!(body.starts_with('{'), "{body}");
        assert!(body.contains(r#""call":"status""#), "{body}");
    }

    #[test]
    fn nothing_is_wrapped_around_the_body() {
        // `family` is a field of the object, not a wrapper around it. See `wire::FAMILY`.
        let out = framed(Wire::Json, &Reply::done()).expect("frames");
        let body: serde_json::Value = serde_json::from_slice(&out[4..]).expect("decodes");
        let keys: Vec<&str> = body
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["ok", "family", "n", "result"], "{body}");
    }

    #[test]
    fn a_message_survives_the_round_trip_blocking() {
        let sent = Call {
            call: "tell".to_owned(),
            args: vec![serde_json::json!("hello")],
            from: Some("magi/main/alpha-rho".to_owned()),
            token: None,
        };
        let mut wire = Vec::new();
        write_to(&mut wire, &sent).expect("writes");
        let back: Call = read_from(&mut wire.as_slice()).expect("reads");
        assert_eq!(sent, back);
    }

    #[tokio::test]
    async fn a_message_survives_the_round_trip_async() {
        let sent = Reply::of(serde_json::json!({"busy": false}));
        let mut wire = Vec::new();
        write(&mut wire, &sent).await.expect("writes");
        let back: Reply = read(&mut wire.as_slice()).await.expect("reads");
        assert_eq!(sent, back);
    }

    #[test]
    fn a_frame_claiming_more_than_the_cap_is_refused_before_anything_is_allocated() {
        let huge = u32::try_from(MOST + 1).expect("fits").to_be_bytes();
        assert!(expecting(huge).is_err());
        assert!(expecting(u32::MAX.to_be_bytes()).is_err());
    }

    #[test]
    fn a_body_that_is_not_json_is_an_error_rather_than_a_panic() {
        let mut wire = 3_u32.to_be_bytes().to_vec();
        wire.extend_from_slice(b"not");
        let read: std::io::Result<Call> = read_from(&mut wire.as_slice());
        assert!(read.is_err());
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    use crate::wire::{Call, Reply};

    #[test]
    fn a_body_says_which_encoding_it_is() {
        assert_eq!(Wire::of(br#"{"call":"listening"}"#), Wire::Json);
        assert_eq!(Wire::of(b"  [1]"), Wire::Json, "after space");
        let cbor = Wire::Cbor
            .encode(&serde_json::json!({"call": "listening"}))
            .expect("encode");
        assert_eq!(Wire::of(&cbor), Wire::Cbor);
    }

    #[test]
    fn both_encodings_carry_the_same_message() {
        let call = Call {
            call: "listening".to_owned(),
            args: vec![serde_json::json!("hello")],
            ..Call::default()
        };
        let as_json = Wire::Json.encode(&call).expect("json");
        let as_cbor = Wire::Cbor.encode(&call).expect("cbor");
        assert_ne!(as_json, as_cbor, "different bytes");
        let back_json: Call = Wire::read_any(&as_json).expect("json");
        let back_cbor: Call = Wire::read_any(&as_cbor).expect("cbor");
        assert_eq!(back_json.call, back_cbor.call, "one shape, two encodings");
        assert_eq!(back_json.args, back_cbor.args);
    }

    #[tokio::test]
    async fn a_frame_is_read_back_in_whichever_it_was_written() {
        for how in [Wire::Json, Wire::Cbor] {
            let mut buffer = Vec::new();
            write_as(&mut buffer, how, &Reply::done())
                .await
                .expect("write");
            let (_reply, seen): (Reply, Wire) =
                read_wire(&mut buffer.as_slice()).await.expect("read");
            assert_eq!(seen, how, "the encoding survived the round trip");
        }
    }
}
