use crate::config::Config;
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
        let keep = match marker {
            0xe0 if payload.starts_with(b"JFIF\0") => config.resolution,
            0xe1 if payload.starts_with(b"Exif\0\0") => config.exif,
            0xe1
                if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0")
                    || payload.starts_with(b"http://ns.adobe.com/xmp/extension/\0") =>
            {
                config.xmp
            }
            0xe2 if payload.starts_with(b"ICC_PROFILE\0") => config.color_profile,
            0xed if payload.starts_with(b"Photoshop 3.0\0") => config.iptc,
            0xeb if payload.starts_with(b"JP") => config.provenance,
            0xfe => config.comments,
            // APP14 Adobe controls color-space interpretation: never discard as unknown.
            0xee if payload.starts_with(b"Adobe") => true,
            0xe0..=0xef => config.unknown,
            _ => true, // image structure, scans, tables and frame parameters
        };
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") && !config.exif {
            if let Some(minimal) = exif::reduce(payload, config)? {
                write_segment(&mut out, marker, &minimal)?;
            }
            removed += 1;
        } else if keep {
            out.extend_from_slice(&input[marker_start..end]);
        } else {
            removed += 1;
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
        c.exif = true;
        let result = clean(&jpeg_fixture(), &c).unwrap();
        assert_eq!(result.removed, 2);
    }

    #[test]
    fn truncated_marker_rejected() {
        assert!(clean(&[0xff, 0xd8, 0xff, 0xe1, 0x00], &Config::default()).is_err());
    }
}
