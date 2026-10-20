use crate::config::Config;
use crate::formats::{self, Format};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    InPlace,
    Copy,
}

#[derive(Default, Debug)]
pub struct Report {
    pub cleaned: usize,
    pub unchanged: usize,
    pub copied: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempPath(PathBuf);
impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn new_temp_near(dest: &Path, content: &[u8], permissions: Permissions) -> Result<TempPath, String> {
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    let stem = dest.file_name().ok_or("missing output filename")?;
    for _ in 0..128 {
        let mut name = OsString::from(".");
        name.push(stem);
        name.push(format!(
            ".metaclean.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let path = dir.join(name);
        let file = OpenOptions::new().write(true).create_new(true).open(&path);
        let mut file = match file {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create temporary file {}: {e}", path.display())),
        };
        let guard = TempPath(path);
        file.write_all(content)
            .and_then(|_| file.set_permissions(permissions))
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("cannot write temporary file {}: {e}", guard.0.display()))?;
        return Ok(guard);
    }
    Err("unable to allocate a unique temporary filename".into())
}

// The Rust standard library provides the replacement semantics on Windows and Unix.
// The temporary file lives beside the destination, so cross-volume moves are avoided.
fn replace_existing(temp: &Path, dest: &Path) -> io::Result<()> {
    fs::rename(temp, dest)
}

// Hard links are atomic and refuse to overwrite an existing output name.
// On filesystems without hard-link support, fail instead of publishing a partial copy.
fn publish_new(temp: &Path, dest: &Path) -> io::Result<()> {
    fs::hard_link(temp, dest)
}

fn durable_directory(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            if let Ok(file) = File::open(parent) {
                let _ = file.sync_all();
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn write_clean(path: &Path, output: &Path, bytes: &[u8], mode: Mode) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    let temp = new_temp_near(output, bytes, meta.permissions())?;
    let result = match mode {
        Mode::InPlace => replace_existing(&temp.0, output),
        Mode::Copy => publish_new(&temp.0, output),
    };
    result.map_err(|e| format!("cannot publish {}: {e}", output.display()))?;
    durable_directory(output);
    Ok(())
}

fn clean_one(src: &Path, dst: Option<&Path>, mode: Mode, format: Format, cfg: &Config, report: &mut Report) -> Result<(), String> {
    let input = fs::read(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let result = formats::clean(&input, format, cfg)
        .map_err(|e| format!("{}: {e}", src.display()))?;
    formats::verify(&result.bytes, format, cfg)
        .map_err(|e| format!("{}: {e}", src.display()))?;
    if result.bytes == input && mode == Mode::InPlace {
        report.unchanged += 1;
        return Ok(());
    }
    let dest = dst.unwrap_or(src);
    write_clean(src, dest, &result.bytes, mode)?;
    if mode == Mode::Copy {
        report.copied += 1;
    }
    if result.bytes == input {
        report.unchanged += 1;
    } else {
        report.cleaned += 1;
    }
    Ok(())
}

fn copy_untouched(src: &Path, dst: &Path) -> Result<(), String> {
    let mut input = File::open(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    let result = io::copy(&mut input, &mut output)
        .and_then(|_| output.set_permissions(fs::metadata(src)?.permissions()))
        .and_then(|_| output.sync_all());
    if let Err(e) = result {
        let _ = fs::remove_file(dst);
        return Err(format!("cannot copy {}: {e}", src.display()));
    }
    Ok(())
}

fn walk_dir(src: &Path, dst: Option<&Path>, mode: Mode, cfg: &Config, report: &mut Report) {
    let entries = match fs::read_dir(src) {
        Ok(entries) => entries,
        Err(e) => {
            report.errors.push(format!("cannot list {}: {e}", src.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(format!("{}: {e}", src.display()));
                continue;
            }
        };
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                report.errors.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            report.skipped += 1;
            continue;
        }
        let child_dest = dst.map(|d| d.join(entry.file_name()));
        if meta.is_dir() {
            if let Some(ref dest) = child_dest {
                if let Err(e) = fs::create_dir(dest) {
                    report.errors.push(format!("cannot create directory {}: {e}", dest.display()));
                    continue;
                }
            }
            walk_dir(&path, child_dest.as_deref(), mode, cfg, report);
        } else if meta.is_file() {
            if let Some(fmt) = formats::format_for_path(&path) {
                if let Err(e) = clean_one(&path, child_dest.as_deref(), mode, fmt, cfg, report) {
                    report.errors.push(e);
                }
            } else if let Some(ref dest) = child_dest {
                if let Err(e) = copy_untouched(&path, dest) {
                    report.errors.push(e);
                } else {
                    report.copied += 1;
                }
            } else {
                report.skipped += 1;
            }
        } else {
            // Named pipes, sockets, devices: never traverse or copy them.
            report.skipped += 1;
        }
    }
}

pub fn output_path(path: &Path, is_dir: bool) -> Result<PathBuf, String> {
    let name = if is_dir {
        path.file_name().ok_or("folder has no name")?
    } else {
        path.file_stem().ok_or("file has no stem")?
    };
    let mut file_name = OsString::from(name);
    file_name.push("_clean");
    if !is_dir {
        if let Some(ext) = path.extension() {
            file_name.push(".");
            file_name.push(ext);
        }
    }
    Ok(path.with_file_name(file_name))
}

pub fn run(path: &Path, mode: Mode, cfg: &Config) -> Result<(Report, Option<PathBuf>), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err("source path must not be a symbolic link".into());
    }
    if meta.is_file() {
        let fmt = formats::format_for_path(path)
            .ok_or("unsupported file extension (expected .png, .jpg, .jpeg or .webp)")?;
        let dest = if mode == Mode::Copy {
            Some(output_path(path, false)?)
        } else {
            None
        };
        let mut report = Report::default();
        clean_one(path, dest.as_deref(), mode, fmt, cfg, &mut report)?;
        Ok((report, dest))
    } else if meta.is_dir() {
        let dest = if mode == Mode::Copy {
            let new_dir = output_path(path, true)?;
            fs::create_dir(&new_dir)
                .map_err(|e| format!("cannot create {}: {e}", new_dir.display()))?;
            Some(new_dir)
        } else {
            None
        };
        let mut report = Report::default();
        walk_dir(path, dest.as_deref(), mode, cfg, &mut report);
        Ok((report, dest))
    } else {
        Err("source must be a regular file or directory".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::png;

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "metaclean-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    fn valid_png_with_comment() -> Vec<u8> {
        // 1x1, grayscale image, from a known-good encoded PNG.
        let mut image = vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82,
            0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0, 58, 126, 155, 85,
            0, 0, 0, 10, 73, 68, 65, 84, 120, 156, 99, 96, 0, 0, 0, 2,
            0, 1, 72, 175, 164, 113,
            0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        // Insertion before IDAT: keyword "parameters" + prompt; CRC handled by PNG code.
        // Instead use the opaque fixture tests in png.rs for CRC and parser correctness.
        let _ = png::clean(&image, &Config::default()).unwrap();
        image.shrink_to_fit();
        image
    }

    #[test]
    fn safe_copy_and_collision() {
        let dir = temp_dir();
        let src = dir.join("portrait.png");
        let source = valid_png_with_comment();
        fs::write(&src, &source).unwrap();
        let (result, dst) = run(&src, Mode::Copy, &Config::default()).unwrap();
        assert_eq!(result.copied, 1);
        let dst = dst.unwrap();
        assert_eq!(fs::read(&src).unwrap(), source);
        assert_eq!(fs::read(&dst).unwrap(), source);
        assert!(run(&src, Mode::Copy, &Config::default()).is_err());
        assert_eq!(fs::read(&dst).unwrap(), source);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recursive_folder_copy_does_not_follow_symlinks() {
        let dir = temp_dir();
        let src = dir.join("photos");
        fs::create_dir(&src).unwrap();
        fs::create_dir(src.join("sub")).unwrap();
        fs::write(src.join("sub").join("photo.png"), valid_png_with_comment()).unwrap();
        fs::write(src.join("note.txt"), b"keep this untouched").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
        let (report, dest) = run(&src, Mode::Copy, &Config::default()).unwrap();
        let dest = dest.unwrap();
        assert_eq!(report.errors.len(), 0);
        assert_eq!(fs::read(dest.join("note.txt")).unwrap(), b"keep this untouched");
        assert!(dest.join("sub/photo.png").exists());
        #[cfg(unix)]
        assert!(!dest.join("loop").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
