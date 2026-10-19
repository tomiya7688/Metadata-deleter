#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Sandbox(pub PathBuf);

impl Sandbox {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "metaclean-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn image(ext: &str) -> Vec<u8> {
    let text = match ext {
        "png" => include_str!("../fixtures/photo.png.hex"),
        "jpg" => include_str!("../fixtures/photo.jpg.hex"),
        "webp" => include_str!("../fixtures/photo.webp.hex"),
        _ => panic!("unknown fixture format"),
    };
    let digits: Vec<_> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    assert_eq!(digits.len() % 2, 0);
    digits.chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
    }).collect()
}

pub fn run(cwd: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_metaclean"))
        .current_dir(cwd)
        .args(arguments)
        .output()
        .expect("execute metaclean")
}

pub fn invoke(cwd: &Path, path: &Path, mode: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_metaclean"))
        .current_dir(cwd)
        .arg(path)
        .arg(mode)
        .args(extra)
        .output()
        .expect("execute metaclean")
}

pub fn success(output: &Output) {
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// The original encoded image bytes must be embedded unchanged in the output.
/// EXIF/XMP/COM tags must not be included in this comparison.
pub fn coded_pixels<'a>(image: &'a [u8], ext: &str) -> &'a [u8] {
    match ext {
        "png" => {
            let mut i = 8;
            while i + 12 <= image.len() {
                let n = u32::from_be_bytes(image[i..i + 4].try_into().unwrap()) as usize;
                let end = i + 12 + n;
                assert!(end <= image.len());
                if &image[i + 4..i + 8] == b"IDAT" {
                    return &image[i + 8..i + 8 + n];
                }
                i = end;
            }
            panic!("PNG fixture missing IDAT");
        }
        "jpg" => {
            let index = image.windows(2).position(|v| v == [0xff, 0xda]).unwrap();
            let length = u16::from_be_bytes([image[index + 2], image[index + 3]]) as usize;
            &image[index + 2 + length..image.len() - 2]
        }
        "webp" => {
            let index = image.windows(4).position(|v| v == b"VP8 ").unwrap();
            let n = u32::from_le_bytes(image[index + 4..index + 8].try_into().unwrap()) as usize;
            &image[index + 8..index + 8 + n]
        }
        _ => unreachable!(),
    }
}
