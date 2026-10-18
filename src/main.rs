mod config;
mod exif;
mod formats;
mod ops;
mod xmp;

use config::{Config, DEFAULT_INI};
use ops::Mode;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = r#"metaclean — local lossless PNG/JPEG/WebP metadata cleaner

Usage:
  metaclean <path> --in-place [--config <file.ini>]
  metaclean <path> --copy     [--config <file.ini>]
  metaclean --print-default-config

--in-place              Update source images via temporary files.
--copy                  Write file_clean.png or directory_clean/ (no overwrite).
--config <file.ini>     Use a custom INI file.
--print-default-config  Print the default INI to stdout.

With no --config, reads ./metaclean.ini when present; otherwise uses built-in defaults.
"#;

struct Args {
    source: PathBuf,
    mode: Mode,
    config: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Args>, String> {
    let mut source = None;
    let mut mode = None;
    let mut config = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--help") | Some("-h") => {
                print!("{USAGE}");
                return Ok(None);
            }
            Some("--version") | Some("-V") => {
                println!("metaclean {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Some("--print-default-config") => {
                print!("{DEFAULT_INI}");
                return Ok(None);
            }
            Some("--in-place") | Some("--copy") => {
                if mode.is_some() {
                    return Err("choose exactly one of --in-place and --copy".into());
                }
                mode = Some(if arg == "--in-place" { Mode::InPlace } else { Mode::Copy });
            }
            Some("--config") => {
                if config.is_some() {
                    return Err("--config specified more than once".into());
                }
                config = Some(PathBuf::from(iter.next().ok_or("--config requires a path")?));
            }
            Some(s) if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            _ => {
                if source.is_some() {
                    return Err("provide exactly one input path".into());
                }
                source = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(Some(Args {
        source: source.ok_or("missing input path")?,
        mode: mode.ok_or("specify --in-place or --copy")?,
        config,
    }))
}

fn execute() -> Result<ExitCode, String> {
    let Some(args) = parse_args(std::env::args_os().skip(1))? else {
        return Ok(ExitCode::SUCCESS);
    };
    let config = if let Some(path) = args.config.as_deref() {
        Config::load(path)?
    } else if Path::new("metaclean.ini").exists() {
        Config::load(Path::new("metaclean.ini"))?
    } else {
        Config::default()
    };
    let (report, output) = ops::run(&args.source, args.mode, &config)?;
    if let Some(path) = output {
        println!("Output: {}", path.display());
    }
    println!(
        "Cleaned: {}, unchanged: {}, copied: {}, skipped: {}, errors: {}",
        report.cleaned, report.unchanged, report.copied, report.skipped, report.errors.len()
    );
    for error in &report.errors {
        eprintln!("error: {error}");
    }
    if report.errors.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn main() -> ExitCode {
    match execute() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_required_and_exclusive() {
        assert!(parse_args(["file.png".into()]).is_err());
        assert!(parse_args(["file.png".into(), "--copy".into(), "--in-place".into()]).is_err());
        assert!(parse_args(["file.png".into(), "--copy".into()]).unwrap().is_some());
    }
}
