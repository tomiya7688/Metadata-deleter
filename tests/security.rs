#![forbid(unsafe_code)]

//! File-input security smoke tests. No external services or untrusted corpus downloads.
//! These tests assert that malformed or unsafe inputs do not clobber user files.

mod support;

use std::fs;
use support::{failure, image, invoke, run, Sandbox};

#[test]
fn truncated_images_do_not_replace_originals() {
    for ext in ["png", "jpg", "webp"] {
        let sb = Sandbox::new("truncate");
        let bytes = image(ext);
        let sample_points = [0, 1, 2, 4, 7, 8, 12, 20, 32, 48, 64, bytes.len() / 2, bytes.len() - 1];
        for (case, &n) in sample_points.iter().enumerate() {
            let n = n.min(bytes.len() - 1);
            let truncated = bytes[..n].to_vec();
            let input = sb.write(&format!("bad-{case}.{ext}"), &truncated);
            failure(&invoke(&sb.0, &input, "--in-place", &[]));
            assert_eq!(fs::read(&input).unwrap(), truncated, "{ext} truncated at {n}");
            assert!(!sb.path(&format!("bad-{case}_clean.{ext}")).exists());
        }
    }
}

#[test]
fn extreme_lengths_invalid_crc_and_malformed_markers_fail_safely() {
    let sb = Sandbox::new("malformed");
    let mut png = image("png");
    png[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut corrupted_png = image("png");
    corrupted_png[29] ^= 0x01;
    let mut jpeg = image("jpg");
    // APP1 segment length too large for the available input.
    jpeg[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
    let mut webp = image("webp");
    webp[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    for (name, content) in [
        ("overflow.png", png),
        ("bad-crc.png", corrupted_png),
        ("oversized.jpg", jpeg),
        ("oversized.webp", webp),
    ] {
        let input = sb.write(name, &content);
        failure(&invoke(&sb.0, &input, "--in-place", &[]));
        assert_eq!(fs::read(input).unwrap(), content, "{name}");
    }
}

#[test]
fn deterministic_malformed_corpus_never_panics_or_changes_source() {
    let sb = Sandbox::new("fuzz-smoke");
    let mut state: u64 = 0xa72f_1a51_9420_3321;
    for ext in ["png", "jpg", "webp"] {
        for number in 0..48 {
            let len = number * 3 + 1;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                // xorshift64 (deterministic corpus across platforms)
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push((state >> 19) as u8);
            }
            // Some candidates reach deeper parser branches with a valid signature.
            let signature: &[u8] = match ext {
                "png" => b"\x89PNG\r\n\x1a\n",
                "jpg" => b"\xff\xd8",
                "webp" => b"RIFF\0\0\0\0WEBP",
                _ => unreachable!(),
            };
            let prefix = signature.len().min(bytes.len());
            bytes[..prefix].copy_from_slice(&signature[..prefix]);
            let name = format!("random-{number}.{ext}");
            let file = sb.write(&name, &bytes);
            let result = invoke(&sb.0, &file, "--in-place", &[]);
            failure(&result);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!stderr.contains("panicked at"), "unexpected panic on {name}: {stderr}");
            assert_eq!(fs::read(&file).unwrap(), bytes, "{name} must be unchanged");
        }
    }
}

#[test]
fn invalid_ini_and_conflicting_flags_do_not_change_input() {
    let sb = Sandbox::new("bad-config");
    let original = image("png");
    let photo = sb.write("photo.png", &original);
    for (case, ini) in [
        "[keep]\ncreator=tomiya\n",
        "[keep]\ncreator='tomiya'\n",
        "[keep]\ncreator=\"unterminated\n",
        "[keep]\ncreator=\"tomiya\" garbage\n",
        "[keep]\nunknown_key=true\n",
        "[keep]\ncreator=\"tomiya\"\nexif=true\n",
    ].iter().enumerate() {
        let config = sb.write(&format!("bad-{case}.ini"), ini.as_bytes());
        failure(&invoke(&sb.0, &photo, "--in-place", &["--config", config.to_str().unwrap()]));
        assert_eq!(fs::read(&photo).unwrap(), original);
    }
    let arg = photo.to_str().unwrap();
    failure(&run(&sb.0, &[arg, "--copy", "--in-place"]));
    assert_eq!(fs::read(photo).unwrap(), original);
}

#[test]
fn existing_copy_target_is_never_overwritten() {
    let sb = Sandbox::new("collision");
    let original = image("png");
    let photo = sb.write("photo.png", &original);
    let dest = sb.write("photo_clean.png", b"do not overwrite");
    failure(&invoke(&sb.0, &photo, "--copy", &[]));
    assert_eq!(fs::read(dest).unwrap(), b"do not overwrite");
    assert_eq!(fs::read(photo).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_at_root_or_inside_folders() {
    use std::os::unix::fs::symlink;
    let sb = Sandbox::new("symlinks");
    let original = image("png");
    let target = sb.write("private.png", &original);
    let link = sb.path("linked.png");
    symlink(&target, &link).unwrap();
    failure(&invoke(&sb.0, &link, "--in-place", &[]));
    assert_eq!(fs::read(&target).unwrap(), original);
    let root = sb.path("directory");
    fs::create_dir_all(&root).unwrap();
    symlink(&target, root.join("linked.png")).unwrap();
    symlink(&root, root.join("loop")).unwrap();
    let copied = invoke(&sb.0, &root, "--copy", &[]);
    assert!(copied.status.success(), "{}", String::from_utf8_lossy(&copied.stderr));
    assert!(!sb.path("directory_clean/linked.png").exists());
    assert!(!sb.path("directory_clean/loop").exists());
    assert_eq!(fs::read(target).unwrap(), original);
}

#[test]
fn unsupported_extension_never_modifies_file() {
    let sb = Sandbox::new("unsupported");
    let bytes = image("png");
    let path = sb.write("image.exe", &bytes);
    failure(&invoke(&sb.0, &path, "--in-place", &[]));
    assert_eq!(fs::read(path).unwrap(), bytes);
}
