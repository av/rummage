//! Search by an uploaded file: images and audio are decoded the same way the indexer
//! decodes them, so an indexed file uploaded again finds itself.

use anyhow::Result;
use rummage_core::EmbedInput;
use rummage_core::service::UploadDecoder;

use super::{audio, image};

pub struct Decoder;

impl UploadDecoder for Decoder {
    fn decode(&self, name: Option<&str>, mime: &str, bytes: &[u8]) -> Option<Result<EmbedInput>> {
        let ext = name
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase());
        if mime.starts_with("image/") {
            return Some(
                image::decode(bytes).map(|img| EmbedInput::Image(image::embed_pixels(&img))),
            );
        }
        if mime.starts_with("audio/") {
            let first = (audio::WINDOW_SECS * audio::RATE as f64) as usize;
            return Some(
                audio::decode(
                    Box::new(std::io::Cursor::new(bytes.to_vec())),
                    ext.as_deref(),
                )
                .map(|mut samples| {
                    // One window: the first 30 s.
                    samples.truncate(first);
                    EmbedInput::Audio {
                        samples_16k_mono: samples,
                    }
                }),
            );
        }
        None
    }
}
