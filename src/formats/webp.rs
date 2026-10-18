use crate::config::Config;
use crate::xmp;
use crate::exif;
use crate::formats::Cleaned;

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) -> Result<(), String> {
    let len = u32::try_from(payload.len()).map_err(|_| "WebP chunk too large")?;
    out.extend_from_slice(kind);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 != 0 {
        out.push(0);
    }
    Ok(())
}

pub fn clean(input: &[u8], config: &Config) -> Result<Cleaned, String> {
    if input.len() < 12 || &input[..4] != b"RIFF" || &input[8..12] != b"WEBP" {
        return Err("not a RIFF WebP file".into());
    }
    let declared = u32::from_le_bytes(input[4..8].try_into().unwrap()) as usize;
    if declared.checked_add(8) != Some(input.len()) {
        return Err("WebP RIFF size mismatch or trailing data".into());
    }
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    let mut i = 12;
    let mut removed = 0;
    let mut wrote_xmp = false;
    let mut seen_image = false;
    let mut seen_vp8x = false;
    while i < input.len() {
        let header = input.get(i..i.saturating_add(8)).ok_or("truncated WebP chunk header")?;
        let kind: [u8; 4] = header[..4].try_into().unwrap();
        let n = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let begin = i.checked_add(8).ok_or("WebP offset overflow")?;
        let end = begin.checked_add(n).ok_or("WebP chunk size overflow")?;
        let padded = end.checked_add(n % 2).ok_or("WebP chunk padding overflow")?;
        let data = input.get(begin..end).ok_or("truncated WebP chunk")?;
        if input.get(end..padded).is_none() {
            return Err("truncated WebP padding".into());
        }
        if n % 2 != 0 && input[end] != 0 {
            return Err("invalid WebP padding".into());
        }
        match &kind {
            b"VP8X" => {
                if seen_vp8x || n != 10 || !chunks.is_empty() {
                    return Err("invalid or misplaced WebP VP8X".into());
                }
                seen_vp8x = true;
            }
            b"VP8 " | b"VP8L" | b"ANMF" => seen_image = true,
            _ => {}
        }
        if &kind == b"EXIF" && !config.exif.keep() {
            if let Some(minimal) = exif::reduce(data, config)? {
                chunks.push((kind, minimal));
            }
            removed += 1;
        } else if &kind == b"XMP " && !config.xmp.keep() {
            if !wrote_xmp {
                if let Some(xml) = xmp::replacement(config) {
                    chunks.push((kind, xml));
                }
                wrote_xmp = true;
            }
            removed += 1;
        } else {
            let keep = match &kind {
                b"EXIF" => config.exif.keep(),
                b"XMP " => config.xmp.keep(),
                b"ICCP" => config.color_profile.keep(),
                b"C2PA" => config.provenance.keep(),
                b"VP8X" | b"VP8 " | b"VP8L" | b"ANIM" | b"ANMF" | b"ALPH" => true,
                _ => config.unknown.keep(),
            };
            if keep {
                chunks.push((kind, data.to_vec()));
            } else {
                removed += 1;
            }
        }
        i = padded;
    }
    if !seen_image || chunks.is_empty() {
        return Err("WebP missing image chunks".into());
    }
    if chunks.iter().any(|(tag, _)| matches!(tag, b"ICCP" | b"EXIF" | b"XMP ")) && !seen_vp8x {
        return Err("WebP metadata requires VP8X header".into());
    }
    let mut present_icc = false;
    let mut present_exif = false;
    let mut present_xmp = false;
    for (tag, _) in &chunks {
        match tag {
            b"ICCP" => present_icc = true,
            b"EXIF" => present_exif = true,
            b"XMP " => present_xmp = true,
            _ => {}
        }
    }
    let mut output = b"RIFF\0\0\0\0WEBP".to_vec();
    for (kind, mut payload) in chunks {
        if &kind == b"VP8X" {
            // ICC, EXIF, XMP flags must agree with the new chunks.
            payload[0] &= !(0x20 | 0x08 | 0x04);
            if present_icc {
                payload[0] |= 0x20;
            }
            if present_exif {
                payload[0] |= 0x08;
            }
            if present_xmp {
                payload[0] |= 0x04;
            }
        }
        push_chunk(&mut output, &kind, &payload)?;
    }
    let size = u32::try_from(output.len() - 8).map_err(|_| "WebP RIFF output too large")?;
    output[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(Cleaned {
        bytes: output,
        removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{verify, Format};

    fn webp_fixture() -> Vec<u8> {
        let mut p = b"RIFF\0\0\0\0WEBP".to_vec();
        push_chunk(&mut p, b"VP8X", &[0x2c, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        push_chunk(&mut p, b"ICCP", b"ICC").unwrap();
        push_chunk(&mut p, b"VP8 ", b"encoded pixels").unwrap();
        push_chunk(&mut p, b"EXIF", b"II*\0\x08\0\0\0\0\0\0\0\0\0").unwrap();
        push_chunk(&mut p, b"XMP ", b"creation history").unwrap();
        let len = (p.len() - 8) as u32;
        p[4..8].copy_from_slice(&len.to_le_bytes());
        p
    }

    #[test]
    fn updates_feature_flags_and_preserves_pixels() {
        let clean = clean(&webp_fixture(), &Config::default()).unwrap();
        assert_eq!(clean.removed, 2);
        assert_eq!(clean.bytes[20], 0x20); // ICC remains, EXIF & XMP removed.
        assert!(clean.bytes.windows(14).any(|w| w == b"encoded pixels"));
        assert!(!clean.bytes.windows(16).any(|w| w == b"creation history"));
        verify(&clean.bytes, Format::Webp, &Config::default()).unwrap();
    }

    #[test]
    fn replaces_xmp_creator_and_keeps_image_bytes() {
        let c = Config::parse("[keep]\ncreator=\"tomiya\"\n").unwrap();
        let clean = clean(&webp_fixture(), &c).unwrap();
        assert!(clean.bytes.windows(6).any(|s| s == b"tomiya"));
        assert!(!clean.bytes.windows(16).any(|s| s == b"creation history"));
        assert!(clean.bytes.windows(14).any(|s| s == b"encoded pixels"));
        assert_eq!(clean.bytes[20], 0x24); // ICC and XMP retained, EXIF removed
        verify(&clean.bytes, Format::Webp, &c).unwrap();
    }

    #[test]
    fn c2pa_riff_chunk_is_classified() {
        let mut p = webp_fixture();
        let mut extra = Vec::new();
        push_chunk(&mut extra, b"C2PA", b"signature and author").unwrap();
        p.extend_from_slice(&extra);
        let declared = (p.len() - 8) as u32;
        p[4..8].copy_from_slice(&declared.to_le_bytes());
        let default = clean(&p, &Config::default()).unwrap();
        assert!(!default.bytes.windows(20).any(|w| w == b"signature and author"));
        let keep = Config::parse("[keep]\nprovenance=true\n").unwrap();
        assert!(clean(&p, &keep).unwrap().bytes.windows(20).any(|w| w == b"signature and author"));
    }

    #[test]
    fn rejects_wrong_riff_size() {
        let mut p = webp_fixture();
        p[4] = 0;
        assert!(clean(&p, &Config::default()).is_err());
    }
}
