//! Minimal TIFF/Exif IFD0 reader and writer. Only non-identifying display fields
//! are reconstructed. We never copy arbitrary tag payloads from the source.
use crate::config::Config;

fn u16_at(data: &[u8], at: usize, little: bool) -> Result<u16, String> {
    let buf: [u8; 2] = data
        .get(at..at.checked_add(2).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF field")?
        .try_into()
        .unwrap();
    Ok(if little {
        u16::from_le_bytes(buf)
    } else {
        u16::from_be_bytes(buf)
    })
}

fn u32_at(data: &[u8], at: usize, little: bool) -> Result<u32, String> {
    let buf: [u8; 4] = data
        .get(at..at.checked_add(4).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF field")?
        .try_into()
        .unwrap();
    Ok(if little {
        u32::from_le_bytes(buf)
    } else {
        u32::from_be_bytes(buf)
    })
}

#[derive(Default)]
struct DisplayFields {
    orientation: Option<u16>,
    x_resolution: Option<(u32, u32)>,
    y_resolution: Option<(u32, u32)>,
    unit: Option<u16>,
}

fn read_display_fields(tiff: &[u8], keep: &Config) -> Result<DisplayFields, String> {
    let little = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err("invalid TIFF byte order".into()),
    };
    if u16_at(tiff, 2, little)? != 42 {
        return Err("invalid TIFF magic".into());
    }
    let ifd = u32_at(tiff, 4, little)? as usize;
    let count = u16_at(tiff, ifd, little)? as usize;
    let entries_start = ifd.checked_add(2).ok_or("TIFF offset overflow")?;
    let entries_end = entries_start
        .checked_add(count.checked_mul(12).ok_or("TIFF entry overflow")?)
        .and_then(|n| n.checked_add(4))
        .ok_or("TIFF IFD overflow")?;
    if tiff.get(entries_start..entries_end).is_none() {
        return Err("truncated TIFF IFD0".into());
    }
    let mut fields = DisplayFields::default();
    for n in 0..count {
        let offset = entries_start + n * 12;
        let tag = u16_at(tiff, offset, little)?;
        let ty = u16_at(tiff, offset + 2, little)?;
        let quantity = u32_at(tiff, offset + 4, little)?;
        let value = offset + 8;
        match (tag, ty, quantity) {
            (0x0112, 3, 1) if keep.orientation => {
                let v = u16_at(tiff, value, little)?;
                if (1..=8).contains(&v) {
                    fields.orientation = Some(v);
                }
            }
            (0x0128, 3, 1) if keep.resolution => {
                let v = u16_at(tiff, value, little)?;
                if v == 1 || v == 2 || v == 3 {
                    fields.unit = Some(v);
                }
            }
            (0x011a | 0x011b, 5, 1) if keep.resolution => {
                let ptr = u32_at(tiff, value, little)? as usize;
                let a = u32_at(tiff, ptr, little)?;
                let b = u32_at(tiff, ptr.checked_add(4).ok_or("TIFF pointer overflow")?, little)?;
                if b != 0 {
                    if tag == 0x011a {
                        fields.x_resolution = Some((a, b));
                    } else {
                        fields.y_resolution = Some((a, b));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(fields)
}

/// Returns None when no display-only EXIF data is needed.
/// Keeps the `Exif\0\0` prefix iff the input had one.
pub fn reduce(payload: &[u8], config: &Config) -> Result<Option<Vec<u8>>, String> {
    let has_prefix = payload.starts_with(b"Exif\0\0");
    let tiff = if has_prefix { &payload[6..] } else { payload };
    let fields = read_display_fields(tiff, config)?;
    let mut entries = Vec::new();
    if let Some(n) = fields.orientation {
        entries.push((0x0112_u16, Field::Short(n)));
    }
    if let Some(n) = fields.x_resolution {
        entries.push((0x011a, Field::Rational(n.0, n.1)));
    }
    if let Some(n) = fields.y_resolution {
        entries.push((0x011b, Field::Rational(n.0, n.1)));
    }
    if let Some(n) = fields.unit {
        entries.push((0x0128, Field::Short(n)));
    }
    if entries.is_empty() {
        return Ok(None);
    }
    // Canonical little-endian TIFF with exactly one IFD and no sub-IFDs.
    let mut out = Vec::new();
    if has_prefix {
        out.extend_from_slice(b"Exif\0\0");
    }
    let mut t = Vec::new();
    t.extend_from_slice(b"II");
    t.extend_from_slice(&42_u16.to_le_bytes());
    t.extend_from_slice(&8_u32.to_le_bytes());
    t.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut data = Vec::new();
    let end_of_ifd = 8 + 2 + (entries.len() * 12) + 4;
    for (tag, field) in &entries {
        t.extend_from_slice(&tag.to_le_bytes());
        match field {
            Field::Short(n) => {
                t.extend_from_slice(&3_u16.to_le_bytes());
                t.extend_from_slice(&1_u32.to_le_bytes());
                t.extend_from_slice(&n.to_le_bytes());
                t.extend_from_slice(&0_u16.to_le_bytes());
            }
            Field::Rational(a, b) => {
                t.extend_from_slice(&5_u16.to_le_bytes());
                t.extend_from_slice(&1_u32.to_le_bytes());
                let offset = u32::try_from(end_of_ifd + data.len())
                    .map_err(|_| "TIFF offset too large")?;
                t.extend_from_slice(&offset.to_le_bytes());
                data.extend_from_slice(&a.to_le_bytes());
                data.extend_from_slice(&b.to_le_bytes());
            }
        }
    }
    t.extend_from_slice(&0_u32.to_le_bytes()); // No thumbnail/other IFD.
    t.extend_from_slice(&data);
    out.extend_from_slice(&t);
    Ok(Some(out))
}

enum Field {
    Short(u16),
    Rational(u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_private_tags_and_keeps_orientation() {
        // Big-endian EXIF with Orientation=6 and Make="ACME".
        let mut t = b"MM\0*\0\0\0\x08\0\x02".to_vec();
        t.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        t.extend_from_slice(&[0x01, 0x0f, 0, 2, 0, 0, 0, 5, 0, 0, 0, 38]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(b"ACME\0");
        let clean = reduce(&t, &Config::default()).unwrap().unwrap();
        assert_eq!(clean.len(), 26);
        assert!(!clean.windows(4).any(|s| s == b"ACME"));
        assert_eq!(read_display_fields(&clean, &Config::default()).unwrap().orientation, Some(6));
        assert_eq!(clean, reduce(&clean, &Config::default()).unwrap().unwrap());
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
