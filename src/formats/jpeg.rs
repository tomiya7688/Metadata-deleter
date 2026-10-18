use crate::config::{Config, Setting};
use crate::xmp;
use crate::exif;
use crate::formats::Cleaned;

fn write_segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) -> Result<(), String> {
    let len = payload.len().checked_add(2).ok_or("JPEG segment too large")?;
    let len = u16::try_from(len).map_err(|_| "JPEG segment too large")?;
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

fn find_scan_end(input: &[u8], mut i: usize) -> Result<usize, String> {
    while i < input.len() {
        if input[i] != 0xff {
            i += 1;
            continue;
        }
        let marker_at = i;
        i += 1;
        while input.get(i) == Some(&0xff) {
            i += 1;
        }
        let b = *input.get(i).ok_or("JPEG entropy scan truncated")?;
        if b == 0x00 || b == 0x01 || (0xd0..=0xd7).contains(&b) {
            // Stuffed FF, TEM or restart marker in entropy-coded data.
            i += 1;
            continue;
        }
        return Ok(marker_at);
    }
    Err("JPEG scan has no terminating marker".into())
}

pub fn clean(input: &[u8], config: &Config) -> Result<Cleaned, String> {
    if !input.starts_with(&[0xff, 0xd8]) {
        return Err("not a JPEG file".into());
    }
    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(&input[..2]);
    let mut i = 2;
    let mut in_scan = false;
    let mut saw_scan = false;
    let mut saw_eoi = false;
    let mut removed = 0;
    let mut wrote_xmp = false;
    let mut saw_mpf = false;

    while i < input.len() {
        if in_scan {
            let end = find_scan_end(input, i)?;
            out.extend_from_slice(&input[i..end]);
            i = end;
            in_scan = false;
        }
        let marker_start = i;
        if input.get(i) != Some(&0xff) {
            return Err("invalid JPEG marker boundary".into());
        }
        while input.get(i) == Some(&0xff) {
            i += 1;
        }
        let marker = *input.get(i).ok_or("truncated JPEG marker")?;
        if marker == 0x00 || marker == 0xd8 {
            return Err("invalid JPEG marker".into());
        }
        i += 1;
        if marker == 0xd9 {
            out.extend_from_slice(&input[marker_start..i]);
            saw_eoi = true;
            // Data after EOI is not part of the JPEG and may hide metadata.
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            out.extend_from_slice(&input[marker_start..i]);
            continue;
        }
        let segment_len = input
            .get(i..i.saturating_add(2))
            .ok_or("truncated JPEG segment size")?;
        let length = u16::from_be_bytes([segment_len[0], segment_len[1]]) as usize;
        if length < 2 {
            return Err("invalid JPEG segment size".into());
        }
        let end = i.checked_add(length).ok_or("JPEG segment size overflow")?;
        let payload = input
            .get(i + 2..end)
            .ok_or("truncated JPEG segment")?;
        let is_exif = marker == 0xe1 && payload.starts_with(b"Exif\0\0");
        let is_xmp = marker == 0xe1 && (payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0")
            || payload.starts_with(b"http://ns.adobe.com/xmp/extension/\0"));
        let is_iptc = marker == 0xed && payload.starts_with(b"Photoshop 3.0\0");
        if marker == 0xe2 && payload.starts_with(b"MPF\0") {
            saw_mpf = true;
        }
        if is_exif && !config.exif.keep() {
            if let Some(minimal) = exif::reduce(payload, config)? {
                write_segment(&mut out, marker, &minimal)?;
            }
            removed += 1;
        } else if is_xmp && !config.xmp.keep() {
            if !wrote_xmp {
                if let Some(xml) = xmp::replacement(config) {
                    let mut packet = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
                    packet.extend_from_slice(&xml);
                    write_segment(&mut out, 0xe1, &packet)?;
                }
                wrote_xmp = true;
            }
            removed += 1;
        } else if is_iptc && !config.iptc.keep() {
            // Replace IPTC attribution with a minimal XMP packet if the input
            // had no XMP replacement yet. Raw Photoshop data is always removed.
            if !wrote_xmp {
                if let Some(xml) = xmp::replacement(config) {
                    let mut packet = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
                    packet.extend_from_slice(&xml);
                    write_segment(&mut out, 0xe1, &packet)?;
                    wrote_xmp = true;
                }
            }
            removed += 1;
        } else if marker == 0xfe {
            match &config.comments {
                Setting::Keep => out.extend_from_slice(&input[marker_start..end]),
                Setting::Remove => removed += 1,
                Setting::Replace(value) => {
                    write_segment(&mut out, marker, value.as_bytes())?;
                    removed += 1;
                }
            }
        } else {
            let keep = match marker {
                0xe0 if payload.starts_with(b"JFIF\0") => config.resolution.keep(),
                0xe0 if payload.starts_with(b"JFXX\0") => config.jpeg_thumbnail.keep(),
                // APP2 MPF is used for multi-picture JPEG: preserve by default.
                0xe2 if payload.starts_with(b"MPF\0") => config.mpf.keep(),
                0xe2 if payload.starts_with(b"FPXR") => config.flashpix.keep(),
                0xec if payload.starts_with(b"Ducky") => config.ducky.keep(),
                0xe1 if is_exif => config.exif.keep(),
                0xe1 if is_xmp => config.xmp.keep(),
                0xe2 if payload.starts_with(b"ICC_PROFILE\0") => config.color_profile.keep(),
                0xed if is_iptc => config.iptc.keep(),
                0xeb if payload.starts_with(b"JP") => config.provenance.keep(),
                0xee if payload.starts_with(b"Adobe") => true,
                0xe0..=0xef => config.unknown.keep(),
                _ => true,
            };
            if keep {
                out.extend_from_slice(&input[marker_start..end]);
            } else {
                removed += 1;
            }
        }
        if marker == 0xda {
            saw_scan = true;
            in_scan = true;
        }
        i = end;
    }
    if !saw_scan || !saw_eoi {
        return Err("JPEG missing scan or EOI marker".into());
    }
    // MPF multi-picture JPEGs can contain offsets into later JPEG images.
    // Removing APP segments or rewriting EXIF can invalidate those offsets.
    // Until MPEntry offsets can be adjusted correctly, refuse the operation
    // rather than silently corrupting multi-picture data.
    if saw_mpf && config.mpf.keep() && out != input {
        return Err("cannot safely rewrite an MPF multi-picture JPEG without repairing image offsets; use --copy with mpf=false only if losing multi-picture content is acceptable".into());
    }
    Ok(Cleaned {
        bytes: out,
        removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{verify, Format};

    fn jpeg_fixture() -> Vec<u8> {
        let mut p = vec![0xff, 0xd8];
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
        exif.extend_from_slice(&0x0112_u16.to_le_bytes());
        exif.extend_from_slice(&3_u16.to_le_bytes());
        exif.extend_from_slice(&1_u32.to_le_bytes());
        exif.extend_from_slice(&6_u16.to_le_bytes());
        exif.extend_from_slice(&0_u16.to_le_bytes());
        exif.extend_from_slice(&0_u32.to_le_bytes());
        write_segment(&mut p, 0xe1, &exif).unwrap();
        write_segment(&mut p, 0xe1, b"http://ns.adobe.com/xap/1.0/\0identifying XMP").unwrap();
        write_segment(&mut p, 0xe2, b"ICC_PROFILE\0display profile").unwrap();
        write_segment(&mut p, 0xfe, b"private comment").unwrap();
        write_segment(&mut p, 0xda, &[1, 1, 0, 0, 63, 0]).unwrap();
        p.extend_from_slice(&[1, 2, 0xff, 0x00, 3, 0xff, 0xd0, 4, 5]);
        p.extend_from_slice(&[0xff, 0xd9]);
        p
    }

    #[test]
    fn preserves_entropy_and_display_fields() {
        let source = jpeg_fixture();
        let clean = clean(&source, &Config::default()).unwrap();
        assert_eq!(clean.removed, 3);
        assert!(!clean.bytes.windows(15).any(|w| w == b"private comment"));
        assert!(!clean.bytes.windows(15).any(|w| w == b"identifying XMP"));
        assert!(clean.bytes.windows(15).any(|w| w == b"display profile"));
        assert!(clean.bytes.windows(9).any(|w| w == [1, 2, 0xff, 0x00, 3, 0xff, 0xd0, 4, 5]));
        verify(&clean.bytes, Format::Jpeg, &Config::default()).unwrap();
    }

    #[test]
    fn can_keep_exif() {
        let mut c = Config::default();
        c.exif = Setting::Keep;
        let result = clean(&jpeg_fixture(), &c).unwrap();
        assert_eq!(result.removed, 2);
    }

    #[test]
    fn replaces_xmp_author_and_com_without_touching_scan() {
        let c = Config::parse("[keep]\ncreator=\"tomiya\"\ncomments=\"safe\"\n").unwrap();
        let clean = clean(&jpeg_fixture(), &c).unwrap();
        assert!(clean.bytes.windows(6).any(|s| s == b"tomiya"));
        assert!(!clean.bytes.windows(15).any(|s| s == b"identifying XMP"));
        assert!(!clean.bytes.windows(15).any(|s| s == b"private comment"));
        assert!(clean.bytes.windows(4).any(|s| s == b"safe"));
        assert!(clean.bytes.windows(9).any(|s| s == [1, 2, 0xff, 0x00, 3, 0xff, 0xd0, 4, 5]));
        verify(&clean.bytes, Format::Jpeg, &c).unwrap();
    }

    #[test]
    fn replaces_exif_artist() {
        let mut p = vec![0xff, 0xd8];
        let mut tiff = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend_from_slice(&0x013b_u16.to_le_bytes());
        tiff.extend_from_slice(&2_u16.to_le_bytes());
        tiff.extend_from_slice(&12_u32.to_le_bytes());
        tiff.extend_from_slice(&26_u32.to_le_bytes());
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff.extend_from_slice(b"Old Account\0");
        write_segment(&mut p, 0xe1, &tiff).unwrap();
        write_segment(&mut p, 0xda, &[1, 1, 0, 0, 63, 0]).unwrap();
        p.extend_from_slice(&[1, 2, 0xff, 0xd9]);
        let c = Config::parse("[keep]\ncreator=\"tomiya\"\n").unwrap();
        let cleaned = clean(&p, &c).unwrap();
        assert!(cleaned.bytes.windows(6).any(|s| s == b"tomiya"));
        assert!(!cleaned.bytes.windows(11).any(|s| s == b"Old Account"));
        verify(&cleaned.bytes, Format::Jpeg, &c).unwrap();
    }

    #[test]
    fn known_app_markers_are_not_treated_as_unknown() {
        let mut jpeg = vec![0xff, 0xd8];
        write_segment(&mut jpeg, 0xe0, b"JFXX\0preview").unwrap();
        write_segment(&mut jpeg, 0xe2, b"MPF\0multi-image info").unwrap();
        write_segment(&mut jpeg, 0xe2, b"FPXR\0flashpix").unwrap();
        write_segment(&mut jpeg, 0xec, b"Ducky photo history").unwrap();
        write_segment(&mut jpeg, 0xda, &[1, 1, 0, 0, 63, 0]).unwrap();
        jpeg.extend_from_slice(&[1, 2, 3, 0xff, 0xd9]);
        assert!(clean(&jpeg, &Config::default()).is_err()); // avoid stale MPF offsets
        let clean_mpf = Config::parse("[keep]\njpeg_thumbnail=true\nmpf=true\nflashpix=true\nducky=true\n").unwrap();
        assert_eq!(clean(&jpeg, &clean_mpf).unwrap().bytes, jpeg);
        let c = Config::parse("[keep]\njpeg_thumbnail=true\nmpf=false\nflashpix=true\nducky=true\n").unwrap();
        let other = clean(&jpeg, &c).unwrap();
        assert!(other.bytes.windows(7).any(|x| x == b"preview"));
        assert!(!other.bytes.windows(16).any(|x| x == b"multi-image info"));
        assert!(other.bytes.windows(8).any(|x| x == b"flashpix"));
        assert!(other.bytes.windows(5).any(|x| x == b"Ducky"));
        verify(&other.bytes, Format::Jpeg, &c).unwrap();
    }

    #[test]
    fn truncated_marker_rejected() {
        assert!(clean(&[0xff, 0xd8, 0xff, 0xe1, 0x00], &Config::default()).is_err());
    }
}
