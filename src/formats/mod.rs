pub mod jpeg;
pub mod png;
pub mod webp;

use crate::config::Config;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

pub fn format_for_path(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some(Format::Png),
        "jpg" | "jpeg" => Some(Format::Jpeg),
        "webp" => Some(Format::Webp),
        _ => None,
    }
}

#[derive(Debug)]
pub struct Cleaned {
    pub bytes: Vec<u8>,
    pub removed: usize,
}

pub fn clean(input: &[u8], format: Format, config: &Config) -> Result<Cleaned, String> {
    match format {
        Format::Png => png::clean(input, config),
        Format::Jpeg => jpeg::clean(input, config),
        Format::Webp => webp::clean(input, config),
    }
}

/// A structural second pass catches accidentally retained removable metadata.
pub fn verify(output: &[u8], format: Format, config: &Config) -> Result<(), String> {
    let second = clean(output, format, config)?;
    if second.bytes != output {
        return Err("post-clean verification failed (non-idempotent output)".into());
    }
    Ok(())
}
