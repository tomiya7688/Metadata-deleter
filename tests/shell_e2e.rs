#![forbid(unsafe_code)]

mod support;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use support::{coded_pixels, contains, failure, image, invoke, success, Sandbox};

fn shell(cwd: &std::path::Path, commands: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_metaclean"))
        .arg("shell")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(commands.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn interactive_shell_accepts_japanese_paths_commas_and_folders() {
    let sb = Sandbox::new("shell-unicode");
    let inputs = ["写真.png", "カンマ,付き.png", "日本語フォルダ/夜景.webp"];
    for name in &inputs {
        let ext = if name.ends_with("webp") {"webp"} else {"png"};
        sb.write(name, &image(ext));
    }
    let output = shell(&sb.0,
        "copy 写真.png, \"カンマ,付き.png\", 日本語フォルダ\nhelp\nquit\n");
    success(&output);
    for (original, cleaned, ext) in [
        ("写真.png", "写真_clean.png", "png"),
        ("カンマ,付き.png", "カンマ,付き_clean.png", "png"),
        ("日本語フォルダ/夜景.webp", "日本語フォルダ_clean/夜景.webp", "webp"),
    ] {
        let original = fs::read(sb.path(original)).unwrap();
        let cleaned = fs::read(sb.path(cleaned)).unwrap();
        assert!(contains(&original, b"OLD_ACCOUNT"));
        assert!(!contains(&cleaned, b"OLD_ACCOUNT"));
        assert_eq!(coded_pixels(&original, ext), coded_pixels(&cleaned, ext));
    }
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("Cleaned: 3"), "{log}");
}

#[test]
fn cli_batch_and_japanese_inplace() {
    let sb = Sandbox::new("cli-multiple-unicode");
    for name in ["沖縄.jpg", "東京.webp"] {
        sb.write(name, &image(name.rsplit('.').next().unwrap()));
    }
    let result = invoke(&sb.0, &sb.path("沖縄.jpg,東京.webp"), "--in-place", &[]);
    success(&result);
    for name in ["沖縄.jpg", "東京.webp"] {
        assert!(!contains(&fs::read(sb.path(name)).unwrap(), b"OLD_ACCOUNT"));
    }
}

#[test]
fn batch_rejects_overlap_and_existing_destination_without_modifying() {
    let sb = Sandbox::new("batch-preflight");
    let original = image("png");
    sb.write("photos/a.png", &original);
    sb.write("others.png", &original);
    let paths = "photos,photos/a.png,others.png";
    let output = invoke(&sb.0, &sb.path(paths), "--in-place", &[]);
    failure(&output);
    assert_eq!(fs::read(sb.path("photos/a.png")).unwrap(), original);
    assert_eq!(fs::read(sb.path("others.png")).unwrap(), original);
    sb.write("others_clean.png", b"do not replace");
    let result = invoke(&sb.0, &sb.path("photos,others.png"), "--copy", &[]);
    failure(&result);
    assert!(!sb.path("photos_clean").exists());
    assert_eq!(fs::read(sb.path("others_clean.png")).unwrap(), b"do not replace");
}

#[test]
fn cli_rejects_invalid_comma_lists() {
    let sb = Sandbox::new("lists");
    sb.write("日本語.png", &image("png"));
    for value in ["日本語.png,", "日本語.png,,other.png", "\"日本語.png"] {
        failure(&invoke(&sb.0, &sb.path(value), "--copy", &[]));
    }
}
