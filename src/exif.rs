//! Safe, bounded TIFF/Exif metadata rewriter. Image data is never decoded.
//! Full EXIF is copied only with exif=true. Otherwise recognized, explicitly
//! allowed fields are rebuilt into canonical little-endian IFDs; all other
//! tags (including MakerNote and the embedded EXIF thumbnail) are discarded.
use crate::config::{Config, Setting};
use std::collections::BTreeMap;

fn u16_at(b: &[u8], at: usize, le: bool) -> Result<u16, String> {
    let v: [u8; 2] = b.get(at..at.checked_add(2).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF u16")?.try_into().unwrap();
    Ok(if le { u16::from_le_bytes(v) } else { u16::from_be_bytes(v) })
}

fn u32_at(b: &[u8], at: usize, le: bool) -> Result<u32, String> {
    let v: [u8; 4] = b.get(at..at.checked_add(4).ok_or("TIFF offset overflow")?)
        .ok_or("truncated TIFF u32")?.try_into().unwrap();
    Ok(if le { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    id: u16,
    ty: u16,
    data: Vec<u8>, // canonical little-endian, including rational components
}

fn entries(tiff: &[u8], offset: usize, le: bool) -> Result<Vec<(usize, u16, u16, usize)>, String> {
    let count = u16_at(tiff, offset, le)? as usize;
    let start = offset.checked_add(2).ok_or("TIFF offset overflow")?;
    let end = start.checked_add(count.checked_mul(12).ok_or("TIFF IFD entry count overflow")?)
        .and_then(|n| n.checked_add(4)).ok_or("TIFF IFD overflow")?;
    if tiff.get(start..end).is_none() { return Err("truncated TIFF IFD".into()); }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = start + i * 12;
        out.push((at, u16_at(tiff, at, le)?, u16_at(tiff, at + 2, le)?, u32_at(tiff, at + 4, le)? as usize));
    }
    Ok(out)
}

fn extract(tiff: &[u8], at: usize, id: u16, ty: u16, count: usize, le: bool) -> Result<Tag, String> {
    let width: usize = match ty {
        1 | 2 | 7 => 1,
        3 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => return Err(format!("unsupported TIFF field type {ty} for tag {id:04x}")),
    };
    let len = count.checked_mul(width).ok_or("TIFF tag length overflow")?;
    let ptr = if len <= 4 { at + 8 } else { u32_at(tiff, at + 8, le)? as usize };
    let end = ptr.checked_add(len).ok_or("TIFF tag data overflow")?;
    let mut data = tiff.get(ptr..end).ok_or("truncated TIFF tag data")?.to_vec();
    if !le {
        let step = match ty { 3 => 2, 4 | 5 | 9 | 10 | 11 => 4, 12 => 8, _ => 1 };
        for part in data.chunks_exact_mut(step) { part.reverse(); }
    }
    Ok(Tag { id, ty, data })
}

fn utf16_tag(id: u16, value: &str) -> Tag {
    let mut data = Vec::new();
    for n in value.encode_utf16().chain(std::iter::once(0)) {
        data.extend_from_slice(&n.to_le_bytes());
    }
    Tag { id, ty: 1, data }
}

fn ascii_tag(id: u16, value: &str) -> Result<Tag, String> {
    if !value.is_ascii() { return Err(format!("EXIF tag {id:04x} needs ASCII; use a Unicode-compatible field")); }
    let mut data = value.as_bytes().to_vec();
    data.push(0);
    Ok(Tag { id, ty: 2, data })
}

/// For data that has a Unicode-compatible Windows XP tag, prefer that tag
/// when the new text cannot be represented as EXIF ASCII.
fn replace_tag(id: u16, value: &str) -> Result<Tag, String> {
    let xp = match id {
        0x013b | 0xa430 | 0x9c9d => Some(0x9c9d), // Artist, CameraOwner, XPAuthor
        0x010e | 0x9c9f => Some(0x9c9f), // Description, XPSubject
        0x9c9b => Some(0x9c9b), // XPTitle
        0x9c9c | 0x9286 => Some(0x9c9c), // XPComment / EXIF UserComment
        0x9c9e => Some(0x9c9e), // XPKeywords
        _ => None,
    };
    if let Some(xp) = xp {
        if (0x9c9b..=0x9c9f).contains(&id) || !value.is_ascii() || id == 0x9286 {
            return Ok(utf16_tag(xp, value));
        }
    }
    ascii_tag(id, value)
}

/// Known TIFF metadata policies. None means the field is discarded rather
/// than copied via the `unknown` setting (which governs outer containers).
fn ifd0_policy<'a>(id: u16, c: &'a Config) -> Option<&'a Setting> {
    match id {
        0x0112 => Some(&c.orientation),
        0x011a | 0x011b | 0x0128 => Some(&c.resolution),
        0x010e | 0x9c9f => Some(&c.description),
        0x010f => Some(&c.camera_make),
        0x0110 => Some(&c.camera_model),
        0x0131 => Some(&c.software),
        0x0132 => Some(&c.timestamps),
        0x013b | 0x9c9d => Some(&c.creator),
        0x8298 => Some(&c.copyright),
        0x9c9b => Some(&c.title),
        0x9c9c => Some(&c.comments),
        0x9c9e => Some(&c.keywords),
        0x4746 | 0x4749 => Some(&c.rating),
        _ => None,
    }
}

fn sub_policy<'a>(id: u16, c: &'a Config) -> Option<&'a Setting> {
    match id {
        // Created/modified dates, offsets and subsecond accuracy.
        0x9003 | 0x9004 | 0x9010..=0x9012 | 0x9290..=0x9292 => Some(&c.timestamps),
        // Camera owner and user-authored comment.
        0xa430 => Some(&c.creator),
        0x9286 => Some(&c.comments),
        // Make/model of lens; serial numbers must be separately opted into.
        0xa433 | 0xa434 => Some(&c.lens),
        0xa431 | 0xa435 => Some(&c.device_serial),
        0xa420 => Some(&c.image_id),
        // Common shutter/exposure/ISO/focal-length settings. MakerNote is
        // deliberately never copied: it may include hidden private data.
        0x829a | 0x829d | 0x8827 | 0x8830..=0x8833
        | 0x9201..=0x920a | 0xa001 | 0xa20e | 0xa20f
        | 0xa401..=0xa407 => Some(&c.camera_settings),
        _ => None,
    }
}

fn set(entries: &mut BTreeMap<u16, Tag>, tag: Tag) {
    entries.insert(tag.id, tag);
}

fn put_selected(
    out: &mut BTreeMap<u16, Tag>, tiff: &[u8], at: usize, id: u16,
    ty: u16, count: usize, le: bool, policy: &Setting,
) -> Result<(), String> {
    match policy {
        Setting::Remove => Ok(()),
        Setting::Keep => {
            // Keep the original value, canonicalized for TIFF byte order.
            set(out, extract(tiff, at, id, ty, count, le)?);
            Ok(())
        }
        Setting::Replace(value) => {
            set(out, replace_tag(id, value)?);
            Ok(())
        }
    }
}

fn write_ifd(tiff: &mut Vec<u8>, tags: &BTreeMap<u16, Tag>) -> Result<usize, String> {
    if tiff.len() % 2 != 0 { tiff.push(0); }
    let start = tiff.len();
    let count = u16::try_from(tags.len()).map_err(|_| "too many EXIF entries")?;
    tiff.extend_from_slice(&count.to_le_bytes());
    let size = tags.len().checked_mul(12).and_then(|n| n.checked_add(4))
        .ok_or("EXIF IFD too large")?;
    tiff.resize(tiff.len().checked_add(size).ok_or("EXIF IFD overflow")?, 0);
    for (index, tag) in tags.values().enumerate() {
        let offset = start + 2 + index * 12;
        tiff[offset..offset + 2].copy_from_slice(&tag.id.to_le_bytes());
        tiff[offset + 2..offset + 4].copy_from_slice(&tag.ty.to_le_bytes());
        let width = match tag.ty { 3 => 2, 4 | 9 | 11 => 4, 5 | 10 | 12 => 8, _ => 1 };
        if tag.data.len() % width != 0 {
            return Err("misaligned EXIF tag value".into());
        }
        let count = u32::try_from(tag.data.len() / width).map_err(|_| "EXIF field too large")?;
        tiff[offset + 4..offset + 8].copy_from_slice(&count.to_le_bytes());
        if tag.data.len() <= 4 {
            tiff[offset + 8..offset + 8 + tag.data.len()].copy_from_slice(&tag.data);
        } else {
            if tiff.len() % 2 != 0 { tiff.push(0); }
            let ptr = u32::try_from(tiff.len()).map_err(|_| "EXIF offset too large")?;
            tiff[offset + 8..offset + 12].copy_from_slice(&ptr.to_le_bytes());
            tiff.extend_from_slice(&tag.data);
        }
    }
    Ok(start)
}

fn patch_pointer(tiff: &mut [u8], ifd: usize, index: usize, target: usize) -> Result<(), String> {
    let offset = ifd + 2 + 12 * index + 8;
    let target = u32::try_from(target).map_err(|_| "EXIF subIFD offset too large")?;
    tiff.get_mut(offset..offset + 4).ok_or("EXIF IFD pointer overflow")?
        .copy_from_slice(&target.to_le_bytes());
    Ok(())
}

/// Rebuild the subset of EXIF requested by the INI. Returns None when there
/// is nothing to retain. JPEG APP1's `Exif\0\0` prefix is preserved.
pub fn reduce(payload: &[u8], cfg: &Config) -> Result<Option<Vec<u8>>, String> {
    let prefix = payload.starts_with(b"Exif\0\0");
    let tiff = if prefix { &payload[6..] } else { payload };
    let le = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err("invalid TIFF byte order".into()),
    };
    if u16_at(tiff, 2, le)? != 42 { return Err("invalid TIFF magic".into()); }
    let root_offset = u32_at(tiff, 4, le)? as usize;
    let mut root = BTreeMap::new();
    let mut sub_offset = None;
    let mut gps_offset = None;
    for (at, id, ty, count) in entries(tiff, root_offset, le)? {
        if id == 0x8769 && ty == 4 && count == 1 {
            sub_offset = Some(u32_at(tiff, at + 8, le)? as usize);
        } else if id == 0x8825 && ty == 4 && count == 1 {
            gps_offset = Some(u32_at(tiff, at + 8, le)? as usize);
        } else if let Some(policy) = ifd0_policy(id, cfg) {
            put_selected(&mut root, tiff, at, id, ty, count, le, policy)?;
        }
    }
    let mut sub = BTreeMap::new();
    if let Some(offset) = sub_offset {
        for (at, id, ty, count) in entries(tiff, offset, le)? {
            if let Some(policy) = sub_policy(id, cfg) {
                if id == 0xa430 {
                    if let Some(value) = policy.replacement() {
                    // Non-ASCII CameraOwnerName becomes XPAuthor at IFD0.
                    if !value.is_ascii() {
                        set(&mut root, utf16_tag(0x9c9d, value));
                        continue;
                    }
                    }
                }
                if id == 0x9286 {
                    if let Some(value) = policy.replacement() {
                    // EXIF UserComment has complex character coding. Replace it
                    // with a standard Unicode XPComment instead of misencoding.
                    set(&mut root, utf16_tag(0x9c9c, value));
                    continue;
                    }
                }
                put_selected(&mut sub, tiff, at, id, ty, count, le, policy)?;
            }
        }
    }
    let mut gps = BTreeMap::new();
    if cfg.gps.keep() {
        if let Some(offset) = gps_offset {
            for (at, id, ty, count) in entries(tiff, offset, le)? {
                // GPSIFD tags are defined in 0x0000..0x001f. Never
                // preserve proprietary unrecognized GPS sub-tags.
                if id <= 0x001f && !(matches!(id, 0x0007 | 0x001d) && !cfg.timestamps.keep()) {
                    set(&mut gps, extract(tiff, at, id, ty, count, le)?);
                }
            }
        }
    }
    if !sub.is_empty() { set(&mut root, Tag { id: 0x8769, ty: 4, data: 0u32.to_le_bytes().to_vec() }); }
    if !gps.is_empty() { set(&mut root, Tag { id: 0x8825, ty: 4, data: 0u32.to_le_bytes().to_vec() }); }
    if root.is_empty() { return Ok(None); }
    let mut result = b"II*\0\x08\0\0\0".to_vec();
    let ifd0 = write_ifd(&mut result, &root)?;
    for (index, id) in root.keys().enumerate() {
        let tags = match *id { 0x8769 => &sub, 0x8825 => &gps, _ => continue };
        let pos = write_ifd(&mut result, tags)?;
        patch_pointer(&mut result, ifd0, index, pos)?;
    }
    if prefix {
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&result);
        Ok(Some(prefixed))
    } else {
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_tiff(root: &[Tag], sub: &[Tag], gps: &[Tag]) -> Vec<u8> {
        let mut a = BTreeMap::new();
        for v in root { set(&mut a, v.clone()); }
        if !sub.is_empty() { set(&mut a, Tag { id: 0x8769, ty: 4, data: vec![0;4] }); }
        if !gps.is_empty() { set(&mut a, Tag { id: 0x8825, ty: 4, data: vec![0;4] }); }
        let mut b = b"II*\0\x08\0\0\0".to_vec();
        let ifd0 = write_ifd(&mut b, &a).unwrap();
        for (i, id) in a.keys().enumerate() {
            let subsource = if *id == 0x8769 { sub } else if *id == 0x8825 { gps } else { continue };
            let mut m = BTreeMap::new();
            for v in subsource { set(&mut m, v.clone()); }
            let pos = write_ifd(&mut b, &m).unwrap();
            patch_pointer(&mut b, ifd0, i, pos).unwrap();
        }
        b
    }

    #[test]
    fn reworks_artist_camera_and_gps_selectively() {
        let t = create_tiff(
            &[ascii_tag(0x013b, "Old Google Account").unwrap(), ascii_tag(0x010f, "Google").unwrap(),
              Tag { id: 0x0112, ty: 3, data: 6u16.to_le_bytes().to_vec() }],
            &[ascii_tag(0xa434, "Old lens").unwrap(), ascii_tag(0xa431, "A1B2-SECRET").unwrap(),
              ascii_tag(0x9003, "2020:01:01 00:00:00").unwrap()],
            &[Tag { id: 0x0002, ty: 5, data: [1u32.to_le_bytes(), 1u32.to_le_bytes()].concat() }]
        );
        let c = Config::default();
        let result = reduce(&t, &c).unwrap().unwrap();
        assert!(!result.windows(6).any(|w| w == b"Google"));
        assert!(!result.windows(6).any(|w| w == b"SECRET"));
        assert!(!result.windows(8).any(|w| w == b"Old lens"));
        assert_eq!(reduce(&result, &c).unwrap().unwrap(), result);
        assert!(result.windows(2).any(|v| v == 6u16.to_le_bytes()));
        let c = Config::parse("[keep]\ncamera_make=true\ngps=true\nlens=true\n").unwrap();
        let kept = reduce(&t, &c).unwrap().unwrap();
        assert!(kept.windows(6).any(|w| w == b"Google"));
        assert!(kept.windows(8).any(|w| w == b"Old lens"));
        assert!(!kept.windows(6).any(|w| w == b"SECRET"));
        assert_eq!(reduce(&kept, &c).unwrap().unwrap(), kept);
    }

    #[test]
    fn replaces_artist_unicode_and_canonicalizes() {
        let t = create_tiff(&[ascii_tag(0x013b, "Old Google Account").unwrap()], &[], &[]);
        let c = Config::parse("[keep]\ncreator=\"富谷\"\n").unwrap();
        let output = reduce(&t, &c).unwrap().unwrap();
        assert!(!output.windows(6).any(|w| w == b"Google"));
        assert!(output.windows(2).any(|w| w == 0x5bcc_u16.to_le_bytes())); // 富
        assert_eq!(reduce(&output, &c).unwrap().unwrap(), output);
    }

    #[test]
    fn malformed_offsets_fail() {
        assert!(reduce(b"Exif\0\0II*\0\xff\xff\xff\xff", &Config::default()).is_err());
    }

    #[test]
    fn rejects_non_ascii_in_strict_ascii_tags() {
        let t = create_tiff(&[ascii_tag(0x010f, "Canon").unwrap()], &[], &[]);
        let c = Config::parse("[keep]\ncamera_make=\"カメラ\"\n").unwrap();
        assert!(reduce(&t, &c).is_err());
    }
}
