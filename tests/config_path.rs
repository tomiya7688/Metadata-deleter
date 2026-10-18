//! Integration test: the actual compiled binary must prefer its adjacent INI.
//! This catches the old bug where the process working directory was consulted.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc: u32 = !0;
    for &byte in kind.iter().chain(data) {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb88320 } else { crc >> 1 };
        }
    }
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&(!crc).to_be_bytes());
    out
}

fn png_with_old_author() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
    bytes.extend_from_slice(&png_chunk(b"tEXt", b"Author\0OLD_ACCOUNT"));
    // 1x1 grayscale pixel, already compressed.
    bytes.extend_from_slice(&png_chunk(b"IDAT", &[120, 156, 99, 96, 0, 0, 0, 2, 0, 1]));
    bytes.extend_from_slice(&png_chunk(b"IEND", b""));
    bytes
}

fn work_folder() -> (PathBuf, Cleanup) {
    let root = std::env::temp_dir().join(format!(
        "metaclean-cli-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("binary")).unwrap();
    fs::create_dir_all(root.join("other_cwd")).unwrap();
    (root.clone(), Cleanup(root))
}

fn run(executable: &Path, cwd: &Path, args: &[&Path]) -> std::process::Output {
    let mut command = Command::new(executable);
    command.current_dir(cwd);
    for arg in args { command.arg(arg); }
    command.output().unwrap()
}

#[test]
fn loads_ini_beside_actual_executable_regardless_of_working_directory() {
    let (root, _cleanup) = work_folder();
    let exe = root.join("binary").join(format!("metaclean{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_metaclean"), &exe).unwrap();
    let cwd = root.join("other_cwd");
    fs::write(root.join("binary/metaclean.ini"), "[keep]\ncreator=\"NEW_AUTHOR\"\n").unwrap();
    fs::write(cwd.join("metaclean.ini"), "[keep]\ncreator=true\n").unwrap();
    let photo = root.join("photo.png");
    fs::write(&photo, png_with_old_author()).unwrap();

    let mode = Path::new("--copy");
    let result = run(&exe, &cwd, &[&photo, mode]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let cleaned = fs::read(root.join("photo_clean.png")).unwrap();
    assert!(cleaned.windows(10).any(|b| b == b"NEW_AUTHOR"));
    assert!(!cleaned.windows(11).any(|b| b == b"OLD_ACCOUNT"));
    assert!(fs::read(&photo).unwrap().windows(11).any(|b| b == b"OLD_ACCOUNT"));

    // An invalid adjacent INI must not silently fall back to CWD or defaults.
    fs::write(root.join("binary/metaclean.ini"), "[keep]\ncreator=tomiya\n").unwrap();
    let failed = run(&exe, &cwd, &[&photo, Path::new("--in-place")]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("must be true, false"));
    assert!(fs::read(photo).unwrap().windows(11).any(|b| b == b"OLD_ACCOUNT"));
}
