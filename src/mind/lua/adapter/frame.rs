use crate::mind::model::RequestFrame;
use crate::mind::provider::{api::Options, model::Model};
use serde::Serialize;
use sha2::{Digest, Sha256};

struct Sink(Sha256);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fingerprint(value: &impl Serialize) -> Option<[u8; 32]> {
    let mut sink = Sink(Sha256::new());
    serde_json::to_writer(&mut sink, value).ok()?;
    Some(sink.0.finalize().into())
}

pub(super) fn observed(
    name: &str,
    model: &Model,
    options: &Options,
    omitted_images: u64,
) -> Option<RequestFrame> {
    Some(RequestFrame {
        version: 1,
        dialect: fingerprint(&name)?,
        policy: fingerprint(&(
            &model.id,
            crate::mind::provider::compat::resolve(model.compat),
            model.accepts_images(),
            model.reasoning,
            &model.thinking,
            model.max_tokens,
            &options.avoid,
        ))?,
        omitted_images,
    })
}
