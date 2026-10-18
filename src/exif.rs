//! Small TIFF/EXIF parser and canonical writer. Image pixels are never decoded.
//! When full EXIF is not retained, only safe display fields and explicitly
//! permitted/replaced textual IFD0 fields are reconstructed.
use crate::config::{Config, Setting};

fn u16_at(data: &[u8], at: usize, little: bool) -> Result<u16, String> {
    let buf: [u8; 2] = data
        .get(at..at.checked_add(2).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF field")?
        .try_into()
        .unwrap();
    Ok(if little { u16::from_le_bytes(buf) } else { u16::from_be_bytes(buf) })
}

fn u32_at(data: &[u8], at: usize, little: bool) -> Result<u32, String> {
    let buf: [u8; 4] = data
        .get(at..at.checked_add(4).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF field")?
        .try_into()
        .unwrap();
    Ok(if little { u32::from_le_bytes(buf) } else { u32::from_be_bytes(buf) })
}

fn ifd_entries(tiff: &[u8], offset: usize, little: bool) -> Result<Vec<(usize, u16, u16, usize)>, String> {
    let count = u16_at(tiff, offset, little)? as usize;
    let start = offset.checked_add(2).ok_or("TIFF offset overflow")?;
    let entries_end = start
        .checked_add(count.checked_mul(12).ok_or("TIFF entry overflow")?)
        .and_then(|n| n.checked_add(4))
        .ok_or("TIFF IFD overflow")?;
    if tiff.get(start..entries_end).is_none() {
        return Err("truncated TIFF IFD".into());
    }
    let mut entries = Vec::with_capacity(count);
    for n in 0..count {
        let at = start + n * 12;
        entries.push((at, u16_at(tiff, at, little)?, u16_at(tiff, at + 2, little)?, u32_at(tiff, at + 4, little)? as usize));
    }
    Ok(entries)
}

fn tag_data<'a>(tiff: &'a [u8], at: usize, ty: u16, count: usize, little: bool) -> Result<&'a [u8], String> {
    let element_size: usize = match ty {
        1 | 2 | 7 => 1,
        3 => 2,
        4 | 9 => 4,
        5 | 10 => 8,
        _ => return Err(format!("unsupported EXIF field type {ty}")),
    };
    let len = count.checked_mul(element_size).ok_or("TIFF field length overflow")?;
    let location = if len <= 4 { at + 8 } else { u32_at(tiff, at + 8, little)? as usize };
    tiff.get(location..location.checked_add(len).ok_or("TIFF data overflow")?)
        .ok_or_else(|| "truncated TIFF field data".into())
}

fn read_text(tiff: &[u8], at: usize, ty: u16, count: usize, little: bool) -> Result<String, String> {
    let bytes = tag_data(tiff, at, ty, count, little)?;
    if ty == 1 && bytes.len() % 2 == 0 { // Microsoft XPAuthor uses UTF-16LE bytes.
        let words: Vec<u16> = bytes.chunks_exact(2).map(|s| u16::from_le_bytes([s[0], s[1]])).collect();
        return String::from_utf16(&words[..words.iter().position(|x| *x == 0).unwrap_or(words.len())])
            .map_err(|_| "invalid XPAuthor UTF-16".into());
    }
    if ty != 2 {
        return Err("unexpected EXIF text encoding".into());
    }
    let until_nul = bytes.iter().position(|c| *c == 0).unwrap_or(bytes.len());
    let text = &bytes[..until_nul];
    if text.is_ascii() {
        return Ok(String::from_utf8(text.to_vec()).unwrap());
    }
    // Non-standard camera encoders sometimes store UTF-8 in TIFF ASCII fields.
    String::from_utf8(text.to_vec()).map_err(|_| "non-UTF-8 EXIF text field cannot be preserved".into())
}

#[derive(Default)]
struct Fields {
    orientation: Option<u16>,
    x_resolution: Option<(u32, u32)>,
    y_resolution: Option<(u32, u32)>,
    unit: Option<u16>,
    creator: Option<String>,
    copyright: Option<String>,
    software: Option<String>,
}

fn choose_text(data: &[u8], at: usize, ty: u16, count: usize, little: bool, policy: &Setting) -> Result<Option<String>, String> {
    match policy {
        Setting::Keep => Ok(Some(read_text(data, at, ty, count, little)?)),
        Setting::Remove => Ok(None),
        Setting::Replace(s) => Ok(Some(s.clone())),
    }
}

fn read_fields(tiff: &[u8], cfg: &Config) -> Result<Fields, String> {
    let little = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err("invalid TIFF byte order".into()),
    };
    if u16_at(tiff, 2, little)? != 42 {
        return Err("invalid TIFF magic".into());
    }
    let ifd = u32_at(tiff, 4, little)? as usize;
    let entries = ifd_entries(tiff, ifd, little)?;
    let mut result = Fields::default();
    let mut xp_author = None;
    let mut exif_ifd = None;
    for (at, tag, ty, count) in entries {
        match (tag, ty, count) {
            (0x0112, 3, 1) if cfg.orientation.keep() => {
                let n = u16_at(tiff, at + 8, little)?;
                if (1..=8).contains(&n) { result.orientation = Some(n); }
            }
            (0x0128, 3, 1) if cfg.resolution.keep() => {
                let n = u16_at(tiff, at + 8, little)?;
                if (1..=3).contains(&n) { result.unit = Some(n); }
            }
            (0x011a | 0x011b, 5, 1) if cfg.resolution.keep() => {
                let data = tag_data(tiff, at, ty, count, little)?;
                let a = u32_at(data, 0, little)?;
                let b = u32_at(data, 4, little)?;
                if b != 0 {
                    if tag == 0x011a { result.x_resolution = Some((a, b)); }
                    else { result.y_resolution = Some((a, b)); }
                }
            }
            (0x0131, _, _) => result.software = choose_text(tiff, at, ty, count, little, &cfg.software)?,
            (0x013b, _, _) => result.creator = choose_text(tiff, at, ty, count, little, &cfg.creator)?,
            (0x8298, _, _) => result.copyright = choose_text(tiff, at, ty, count, little, &cfg.copyright)?,
            (0x9c9d, 1, _) => xp_author = choose_text(tiff, at, ty, count, little, &cfg.creator)?,
            (0x8769, 4, 1) if !matches!(cfg.creator, Setting::Remove) => {
                exif_ifd = Some(u32_at(tiff, at + 8, little)? as usize);
            }
            _ => {}
        }
    }
    if result.creator.is_none() {
        result.creator = xp_author;
    }
    // CameraOwnerName is in the Exif SubIFD and is treated as a creator alias.
    if result.creator.is_none() {
        if let Some(offset) = exif_ifd {
            for (at, tag, ty, count) in ifd_entries(tiff, offset, little)? {
                if tag == 0xa430 {
                    result.creator = choose_text(tiff, at, ty, count, little, &cfg.creator)?;
                    break;
                }
            }
        }
    }
    Ok(result)
}

enum Field {
    Short(u16),
    Rational(u32, u32),
    Ascii(String),
    Bytes(Vec<u8>),
}

fn text_field(text: String, name: &str) -> Result<Field, String> {
    if text.contains('\0') {
        return Err(format!("EXIF {name} includes NUL"));
    }
    if !text.is_ascii() {
        return Err(format!("EXIF {name} is not ASCII; use a Unicode-capable metadata field"));
    }
    Ok(Field::Ascii(text))
}

/// Returns None if the source has no allowed text/display EXIF fields.
/// Keeps the `Exif\0\0` prefix iff it was present in the input.
pub fn reduce(payload: &[u8], config: &Config) -> Result<Option<Vec<u8>>, String> {
    let prefix = payload.starts_with(b"Exif\0\0");
    let tiff = if prefix { &payload[6..] } else { payload };
    let fields = read_fields(tiff, config)?;
    let mut entries = Vec::new();
    if let Some(n) = fields.orientation { entries.push((0x0112_u16, Field::Short(n))); }
    if let Some(n) = fields.x_resolution { entries.push((0x011a, Field::Rational(n.0, n.1))); }
    if let Some(n) = fields.y_resolution { entries.push((0x011b, Field::Rational(n.0, n.1))); }
    if let Some(n) = fields.unit { entries.push((0x0128, Field::Short(n))); }
    if let Some(text) = fields.software { entries.push((0x0131, text_field(text, "Software")?)); }
    if let Some(text) = fields.creator {
        if text.is_ascii() {
            entries.push((0x013b, text_field(text, "Artist")?));
        } else {
            // XPAuthor is a standardized UTF-16LE Windows EXIF field.
            let mut bytes = Vec::new();
            for word in text.encode_utf16().chain(std::iter::once(0)) {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            entries.push((0x9c9d, Field::Bytes(bytes)));
        }
    }
    if let Some(text) = fields.copyright { entries.push((0x8298, text_field(text, "Copyright")?)); }
    if entries.is_empty() { return Ok(None); }
    entries.sort_by_key(|(tag, _)| *tag);
    let ifd_size = 8usize + 2 + entries.len() * 12 + 4;
    let mut out = Vec::new();
    if prefix { out.extend_from_slice(b"Exif\0\0"); }
    let mut t = b"II*\0\x08\0\0\0".to_vec();
    t.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut data = Vec::new();
    for (tag, field) in entries {
        t.extend_from_slice(&tag.to_le_bytes());
        let (ty, content) = match field {
            Field::Short(n) => (3u16, n.to_le_bytes().to_vec()),
            Field::Rational(a, b) => {
                let mut v = a.to_le_bytes().to_vec();
                v.extend_from_slice(&b.to_le_bytes());
                (5, v)
            }
            Field::Ascii(text) => {
                let mut v = text.into_bytes();
                v.push(0);
                (2, v)
            }
            Field::Bytes(bytes) => (1, bytes),
        };
        t.extend_from_slice(&ty.to_le_bytes());
        let count = if ty == 5 { 1 } else if ty == 3 { 1 } else { u32::try_from(content.len()).map_err(|_| "EXIF text too large")? };
        t.extend_from_slice(&count.to_le_bytes());
        if content.len() <= 4 {
            t.extend_from_slice(&content);
            t.resize(t.len() + 4 - content.len(), 0);
        } else {
            let offset = u32::try_from(ifd_size.checked_add(data.len()).ok_or("TIFF offset overflow")?)
                .map_err(|_| "TIFF offset too large")?;
            t.extend_from_slice(&offset.to_le_bytes());
            data.extend_from_slice(&content);
            if data.len() % 2 != 0 { data.push(0); }
        }
    }
    t.extend_from_slice(&0u32.to_le_bytes());
    t.extend_from_slice(&data);
    out.extend_from_slice(&t);
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_with_artist() -> Vec<u8> {
        // Little-endian TIFF: Artist="Old Google Account", Orientation=6.
        let mut t = b"II*\0\x08\0\0\0\x02\0".to_vec();
        t.extend_from_slice(&0x0112_u16.to_le_bytes());
        t.extend_from_slice(&3_u16.to_le_bytes());
        t.extend_from_slice(&1_u32.to_le_bytes());
        t.extend_from_slice(&6_u16.to_le_bytes());
        t.extend_from_slice(&0_u16.to_le_bytes());
        t.extend_from_slice(&0x013b_u16.to_le_bytes());
        t.extend_from_slice(&2_u16.to_le_bytes());
        t.extend_from_slice(&19_u32.to_le_bytes());
        t.extend_from_slice(&38_u32.to_le_bytes());
        t.extend_from_slice(&0_u32.to_le_bytes());
        t.extend_from_slice(b"Old Google Account\0");
        t
    }

    #[test]
    fn strips_private_tags_and_keeps_orientation() {
        let t = fixture_with_artist();
        let clean = reduce(&t, &Config::default()).unwrap().unwrap();
        assert!(!clean.windows(6).any(|s| s == b"Google"));
        assert_eq!(read_fields(&clean, &Config::default()).unwrap().orientation, Some(6));
        assert_eq!(clean, reduce(&clean, &Config::default()).unwrap().unwrap());
    }

    #[test]
    fn replaces_old_artist_and_is_idempotent() {
        let mut c = Config::parse("[keep]\ncreator = \"tomiya\"\n").unwrap();
        let clean = reduce(&fixture_with_artist(), &c).unwrap().unwrap();
        assert!(!clean.windows(6).any(|s| s == b"Google"));
        assert_eq!(read_fields(&clean, &c).unwrap().creator.as_deref(), Some("tomiya"));
        assert_eq!(clean, reduce(&clean, &c).unwrap().unwrap());
        c.creator = Setting::Keep;
        assert_eq!(read_fields(&clean, &c).unwrap().creator.as_deref(), Some("tomiya"));
    }

    #[test]
    fn unicode_creator_uses_xp_author_utf16() {
        let c = Config::parse("[keep]\ncreator = \"富谷\"\n").unwrap();
        let clean = reduce(&fixture_with_artist(), &c).unwrap().unwrap();
        assert_eq!(read_fields(&clean, &c).unwrap().creator.as_deref(), Some("富谷"));
        assert_eq!(clean, reduce(&clean, &c).unwrap().unwrap());
    }

    #[test]
    fn blank_tiff_strips_entire_exif() {
        let t = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
        assert!(reduce(t, &Config::default()).unwrap().is_none());
    }

    #[test]
    fn malformed_offsets_rejected() {
        assert!(reduce(b"Exif\0\0II*\0\xff\xff\xff\xff", &Config::default()).is_err());
    }
}
