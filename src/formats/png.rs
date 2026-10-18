use crate::config::{Config, Setting};
use crate::xmp;
use crate::exif;
use crate::formats::Cleaned;

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

const fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb88320
            } else {
                crc >> 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = crc_table();

fn crc32(bytes: &[u8]) -> u32 {
    let mut state: u32 = !0;
    for &b in bytes {
        state = (state >> 8) ^ CRC_TABLE[((state as u8) ^ b) as usize];
    }
    !state
}

fn add_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) -> Result<(), String> {
    let size = u32::try_from(payload.len()).map_err(|_| "PNG chunk too large")?;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let mut crc_input = Vec::with_capacity(4 + payload.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(payload);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    Ok(())
}

fn add_itxt(out: &mut Vec<u8>, keyword: &[u8], text: &[u8]) -> Result<(), String> {
    let mut data = Vec::new();
    data.extend_from_slice(keyword);
    // iTXt: keyword NUL, uncompressed flag & method, empty language & translation.
    data.extend_from_slice(&[0, 0, 0, 0, 0]);
    data.extend_from_slice(text);
    add_chunk(out, b"iTXt", &data)
}

fn keep_known(kind: &[u8; 4], config: &Config) -> Option<bool> {
    match kind {
        b"iCCP" | b"sRGB" | b"cHRM" | b"gAMA" | b"cICP" => Some(config.color_profile.keep()),
        b"pHYs" => Some(config.resolution.keep()),
        b"tIME" => Some(config.timestamps.keep()),
        // C2PA/JUMBF provenance is stored in caBX, not in a PNG text chunk.
        b"caBX" => Some(config.provenance.keep()),
        // Public/commonly used non-rendering extension chunks, separate from unknown.
        b"oFFs" | b"pCAL" | b"sCAL" | b"vpAg" => Some(config.png_extension.keep()),
        // APNG, transparency, palette, and rendering-related chunks are preserved.
        b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" | b"bKGD" | b"sBIT"
        | b"hIST" | b"sPLT" | b"sTER" | b"acTL" | b"fcTL" | b"fdAT" => Some(true),
        _ => None,
    }
}

pub fn clean(input: &[u8], config: &Config) -> Result<Cleaned, String> {
    if !input.starts_with(SIGNATURE) {
        return Err("not a PNG file".into());
    }
    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(SIGNATURE);
    let mut i = 8_usize;
    let mut index = 0;
    let mut seen_ihdr = false;
    let mut seen_idat = false;
    let mut seen_iend = false;
    let mut removed = 0;
    let mut wrote_xmp = false;
    while i < input.len() {
        let chunk_start = i;
        let length_bytes: [u8; 4] = input
            .get(i..i.saturating_add(4))
            .ok_or("truncated PNG chunk length")?
            .try_into()
            .unwrap();
        let length = u32::from_be_bytes(length_bytes) as usize;
        i = i.checked_add(4).ok_or("PNG offset overflow")?;
        let kind: [u8; 4] = input
            .get(i..i.saturating_add(4))
            .ok_or("truncated PNG chunk type")?
            .try_into()
            .unwrap();
        if !kind.iter().all(|b| b.is_ascii_alphabetic()) {
            return Err("invalid PNG chunk type".into());
        }
        if index == 0 && &kind != b"IHDR" {
            return Err("PNG must start with IHDR".into());
        }
        if &kind == b"IHDR" {
            if seen_ihdr || length != 13 {
                return Err("invalid or duplicate PNG IHDR".into());
            }
            seen_ihdr = true;
        }
        i += 4;
        let end_data = i.checked_add(length).ok_or("PNG chunk size overflow")?;
        let end = end_data.checked_add(4).ok_or("PNG chunk CRC overflow")?;
        let data = input.get(i..end_data).ok_or("truncated PNG chunk data")?;
        let expected_bytes: [u8; 4] = input
            .get(end_data..end)
            .ok_or("truncated PNG chunk CRC")?
            .try_into()
            .unwrap();
        let expected = u32::from_be_bytes(expected_bytes);
        if crc32(&input[chunk_start + 4..end_data]) != expected {
            return Err(format!("PNG CRC mismatch in {}", String::from_utf8_lossy(&kind)));
        }
        if &kind == b"IDAT" {
            seen_idat = true;
        }
        if &kind == b"IEND" {
            if length != 0 || !seen_idat || end != input.len() {
                return Err("invalid PNG IEND or trailing data".into());
            }
            seen_iend = true;
        }
        if matches!(&kind, b"tEXt" | b"iTXt" | b"zTXt") {
            let nul = data.iter().position(|b| *b == 0).ok_or("invalid PNG text keyword")?;
            if nul == 0 || nul > 79 {
                return Err("invalid PNG text keyword length".into());
            }
            let keyword = &data[..nul];
            let key = String::from_utf8_lossy(keyword).to_ascii_lowercase();
            let is_xmp = matches!(key.trim(), "xml:com.adobe.xmp" | "raw profile type xmp" | "xmp");
            // A replacement must not retain the original XMP, which could embed
            // the previous name in several additional fields.
            if is_xmp && !config.xmp.keep() {
                if !wrote_xmp {
                    if let Some(xml) = xmp::replacement(config) {
                        add_itxt(&mut output, b"XML:com.adobe.xmp", &xml)?;
                    }
                    wrote_xmp = true;
                }
                removed += 1;
            } else {
                match config.png_keyword(keyword) {
                    Setting::Keep => output.extend_from_slice(&input[chunk_start..end]),
                    Setting::Remove => removed += 1,
                    Setting::Replace(value) => {
                        add_itxt(&mut output, keyword, value.as_bytes())?;
                        removed += 1;
                    }
                }
            }
        } else if &kind == b"eXIf" && !config.exif.keep() {
            if let Some(minimal) = exif::reduce(data, config)? {
                add_chunk(&mut output, &kind, &minimal)?;
            }
            removed += 1;
        } else {
            let keep = if &kind == b"eXIf" {
                config.exif.keep()
            } else if let Some(known) = keep_known(&kind, config) {
                known
            } else if kind[0].is_ascii_uppercase() {
                return Err(format!("unknown critical PNG chunk {}", String::from_utf8_lossy(&kind)));
            } else {
                config.unknown.keep()
            };
            if keep {
                output.extend_from_slice(&input[chunk_start..end]);
            } else {
                removed += 1;
            }
        }
        i = end;
        index += 1;
        if seen_iend {
            break;
        }
    }
    if !seen_ihdr || !seen_idat || !seen_iend {
        return Err("incomplete PNG file".into());
    }
    Ok(Cleaned { bytes: output, removed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{verify, Format};

    fn png_fixture() -> Vec<u8> {
        let mut p = SIGNATURE.to_vec();
        add_chunk(&mut p, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]).unwrap();
        add_chunk(&mut p, b"tEXt", b"parameters\0private prompt").unwrap();
        add_chunk(&mut p, b"tEXt", b"Title\0Sunset").unwrap();
        add_chunk(&mut p, b"iCCP", b"sample ICC").unwrap();
        add_chunk(&mut p, b"IDAT", b"opaque compressed pixels").unwrap();
        add_chunk(&mut p, b"IEND", b"").unwrap();
        p
    }

    #[test]
    fn removes_only_private_text() {
        let src = png_fixture();
        let clean = clean(&src, &Config::default()).unwrap();
        assert_eq!(clean.removed, 1);
        assert!(!clean.bytes.windows(14).any(|x| x == b"private prompt"));
        assert!(clean.bytes.windows(6).any(|x| x == b"Sunset"));
        assert!(clean.bytes.windows(10).any(|x| x == b"sample ICC"));
        assert!(clean.bytes.windows(24).any(|x| x == b"opaque compressed pixels"));
        verify(&clean.bytes, Format::Png, &Config::default()).unwrap();
    }

    #[test]
    fn replaces_creator_and_quoted_text_without_reencoding_pixels() {
        let mut p = SIGNATURE.to_vec();
        add_chunk(&mut p, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]).unwrap();
        add_chunk(&mut p, b"tEXt", b"Author\0Old Google Account").unwrap();
        add_chunk(&mut p, b"zTXt", b"Creator\0\0compressed old value").unwrap();
        add_chunk(&mut p, b"tEXt", b"parameters\0private prompt").unwrap();
        add_chunk(&mut p, b"IDAT", b"opaque compressed pixels").unwrap();
        add_chunk(&mut p, b"IEND", b"").unwrap();
        let c = Config::parse("[keep]\ncreator=\"tomiya\"\nai_generation=\"new prompt\"\n").unwrap();
        let cleaned = clean(&p, &c).unwrap();
        assert!(cleaned.bytes.windows(6).any(|s| s == b"tomiya"));
        assert!(cleaned.bytes.windows(10).any(|s| s == b"new prompt"));
        assert!(!cleaned.bytes.windows(6).any(|s| s == b"Google"));
        assert!(!cleaned.bytes.windows(14).any(|s| s == b"private prompt"));
        assert!(cleaned.bytes.windows(24).any(|s| s == b"opaque compressed pixels"));
        verify(&cleaned.bytes, Format::Png, &c).unwrap();
    }

    #[test]
    fn replaces_xmp_with_only_new_author() {
        let mut p = png_fixture();
        let at = p.len()-12;
        let mut xmp_chunk = Vec::new();
        add_chunk(&mut xmp_chunk, b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<creator>OLD_ACCOUNT</creator>").unwrap();
        p.splice(at..at, xmp_chunk);
        let c = Config::parse("[keep]\ncreator=\"tomiya\"\n").unwrap();
        let cleaned = clean(&p, &c).unwrap();
        assert!(!cleaned.bytes.windows(11).any(|s| s == b"OLD_ACCOUNT"));
        assert!(cleaned.bytes.windows(6).any(|s| s == b"tomiya"));
        verify(&cleaned.bytes, Format::Png, &c).unwrap();
    }

    #[test]
    fn common_text_categories_and_png_extensions_obey_ini() {
        let mut p = SIGNATURE.to_vec();
        add_chunk(&mut p, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]).unwrap();
        add_chunk(&mut p, b"tEXt", b"Source\0Old device").unwrap();
        add_chunk(&mut p, b"tEXt", b"Title\0Original title").unwrap();
        add_chunk(&mut p, b"tEXt", b"Email\0private@example.com").unwrap();
        add_chunk(&mut p, b"tEXt", b"GPS\0secret coordinate").unwrap();
        add_chunk(&mut p, b"oFFs", b"position").unwrap();
        add_chunk(&mut p, b"IDAT", b"pixels").unwrap();
        add_chunk(&mut p, b"IEND", b"").unwrap();
        let cfg = Config::parse("[keep]\ntitle=\"New title\"\npng_extension=false\n").unwrap();
        let clean = clean(&p, &cfg).unwrap();
        for secret in ["Old device", "private@example.com", "secret coordinate", "Original title", "position"] {
            assert!(!String::from_utf8_lossy(&clean.bytes).contains(secret), "{secret}");
        }
        assert!(clean.bytes.windows(9).any(|w| w == b"New title"));
        assert!(clean.bytes.windows(6).any(|w| w == b"pixels"));
        verify(&clean.bytes, Format::Png, &cfg).unwrap();
    }

    #[test]
    fn c2pa_provenance_not_treated_as_unknown() {
        let mut p = SIGNATURE.to_vec();
        add_chunk(&mut p, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]).unwrap();
        add_chunk(&mut p, b"caBX", b"signed old identity").unwrap();
        add_chunk(&mut p, b"IDAT", b"pixels").unwrap();
        add_chunk(&mut p, b"IEND", b"").unwrap();
        let cleaned = clean(&p, &Config::default()).unwrap();
        assert!(!cleaned.bytes.windows(19).any(|x| x == b"signed old identity"));
        let c = Config::parse("[keep]\nprovenance=true\n").unwrap();
        assert!(clean(&p, &c).unwrap().bytes.windows(19).any(|x| x == b"signed old identity"));
    }

    #[test]
    fn rejects_corrupt_crc() {
        let mut p = png_fixture();
        p[35] ^= 1;
        assert!(clean(&p, &Config::default()).is_err());
    }

    #[test]
    fn config_can_keep_generation_data() {
        let mut conf = Config::default();
        conf.ai_generation = Setting::Keep;
        assert_eq!(clean(&png_fixture(), &conf).unwrap().removed, 0);
    }
}
