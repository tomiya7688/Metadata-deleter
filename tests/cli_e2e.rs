#![forbid(unsafe_code)]

mod support;

use std::fs;
use support::{coded_pixels, contains, failure, image, invoke, run, success, Sandbox};

#[test]
fn file_copy_and_in_place_all_supported_formats_preserve_pixels() {
    for ext in ["png", "jpg", "webp"] {
        let sandbox = Sandbox::new(ext);
        let name = format!("photo.{ext}");
        let original = image(ext);
        let input = sandbox.write(&name, &original);
        let result = invoke(&sandbox.0, &input, "--copy", &[]);
        success(&result);
        let copied = sandbox.path(&format!("photo_clean.{ext}"));
        let cleaned = fs::read(&copied).unwrap();
        assert!(!contains(&cleaned, b"OLD_ACCOUNT"), "{ext} must strip personal data");
        assert_eq!(coded_pixels(&original, ext), coded_pixels(&cleaned, ext), "{ext}");
        assert_eq!(fs::read(&input).unwrap(), original, "copy must preserve input");

        let collision = invoke(&sandbox.0, &input, "--copy", &[]);
        failure(&collision);
        assert_eq!(fs::read(&copied).unwrap(), cleaned, "must not overwrite copy");

        let inplace = invoke(&sandbox.0, &input, "--in-place", &[]);
        success(&inplace);
        assert_eq!(fs::read(&input).unwrap(), cleaned);
        let repeat = invoke(&sandbox.0, &input, "--in-place", &[]);
        success(&repeat);
        assert!(String::from_utf8_lossy(&repeat.stdout).contains("unchanged: 1"));
        assert_eq!(fs::read(&input).unwrap(), cleaned);
    }
}

#[test]
fn recursive_folder_copy_and_in_place_with_unknown_files() {
    let sb = Sandbox::new("folders");
    for ext in ["png", "jpg", "webp"] {
        sb.write(&format!("photos/nested/a.{ext}"), &image(ext));
    }
    sb.write("photos/note.txt", b"text unchanged");
    let input = sb.path("photos");
    success(&invoke(&sb.0, &input, "--copy", &[]));
    assert_eq!(fs::read(sb.path("photos_clean/note.txt")).unwrap(), b"text unchanged");
    for ext in ["png", "jpg", "webp"] {
        let src = fs::read(sb.path(&format!("photos/nested/a.{ext}"))).unwrap();
        let cleaned = fs::read(sb.path(&format!("photos_clean/nested/a.{ext}"))).unwrap();
        assert!(contains(&src, b"OLD_ACCOUNT"));
        assert!(!contains(&cleaned, b"OLD_ACCOUNT"));
        assert_eq!(coded_pixels(&src, ext), coded_pixels(&cleaned, ext));
    }
    failure(&invoke(&sb.0, &input, "--copy", &[]));
    success(&invoke(&sb.0, &input, "--in-place", &[]));
    for ext in ["png", "jpg", "webp"] {
        assert!(!contains(&fs::read(sb.path(&format!("photos/nested/a.{ext}"))).unwrap(), b"OLD_ACCOUNT"));
    }
    assert_eq!(fs::read(sb.path("photos/note.txt")).unwrap(), b"text unchanged");
}

#[test]
fn explicit_ini_supports_keep_delete_and_double_quoted_replacement() {
    let sb = Sandbox::new("ini");
    let original = image("png");
    let input = sb.write("old.png", &original);
    let config = sb.write("overrides.ini", b"[keep]\ncreator = \"tomiya\"\nai_generation = true\n");
    let result = invoke(&sb.0, &input, "--copy", &["--config", config.to_str().unwrap()]);
    success(&result);
    let output = fs::read(sb.path("old_clean.png")).unwrap();
    assert!(!contains(&output, b"OLD_ACCOUNT"));
    assert!(contains(&output, b"tomiya"));
    assert!(contains(&output, b"secret prompt"), "true must preserve AI generation data");
    assert_eq!(coded_pixels(&original, "png"), coded_pixels(&output, "png"));
}

#[test]
fn command_line_help_version_and_defaults() {
    let sb = Sandbox::new("cli");
    for option in ["--help", "--version", "--print-default-config"] {
        let output = run(&sb.0, &[option]);
        success(&output);
        assert!(!output.stdout.is_empty());
    }
    let defaults = run(&sb.0, &["--print-default-config"]);
    let ini = String::from_utf8(defaults.stdout).unwrap();
    assert!(ini.contains("[keep]"));
    assert!(ini.contains("creator = false"));
    for args in [vec!["--copy"], vec!["--not-an-option"], vec!["--config"]] {
        failure(&run(&sb.0, &args));
    }
}
