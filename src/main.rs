#![forbid(unsafe_code)]

use metaclean::config::{load_config, DEFAULT_INI};
use metaclean::ops::Mode;
use metaclean::shell::{parse_paths, run_batch, run_repl};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = r#"metaclean — local lossless PNG/JPEG/WebP metadata cleaner

Usage:
  metaclean                            Open the interactive shell
  metaclean shell [--config file.ini]  Open the interactive shell
  metaclean <path1,path2,...> --copy [--config file.ini]
  metaclean <path1,path2,...> --in-place [--config file.ini]
  metaclean --print-default-config

Paths can be files or folders (recursively processed). For filenames with
commas, quote the path INSIDE the list: '"photo,one.png",photo2.png'.
Separate quoted arguments also work; do not use shell expansion/eval.
Japanese filenames and directories are supported.

Default config: metaclean.ini next to the executable, not the working directory.
Explicit --config takes precedence. No --config and no adjacent INI = defaults.
"#;

struct Args {
    sources: Vec<PathBuf>,
    mode: Option<Mode>,
    config: Option<PathBuf>,
    shell: bool,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Args>, String> {
    let mut sources = Vec::new();
    let mut mode = None;
    let mut config = None;
    let mut shell = false;
    let mut pending_paths = String::new();
    let mut iter = args.into_iter().peekable();
    if let Some(arg) = iter.peek() {
        if arg == "shell" || arg == "--shell" {
            shell = true;
            iter.next();
        }
    } else {
        shell = true;
    }
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--help") | Some("-h") => { print!("{USAGE}"); return Ok(None); }
            Some("--version") | Some("-V") => {
                println!("metaclean {}", env!("CARGO_PKG_VERSION")); return Ok(None);
            }
            Some("--print-default-config") => { print!("{DEFAULT_INI}"); return Ok(None); }
            Some("--in-place") | Some("--copy") if !shell => {
                if mode.is_some() { return Err("choose exactly one of --in-place and --copy".into()); }
                mode = Some(if arg == "--in-place" { Mode::InPlace } else { Mode::Copy });
            }
            Some("--config") => {
                if config.is_some() { return Err("--config specified more than once".into()); }
                config = Some(PathBuf::from(iter.next().ok_or("--config requires a path")?));
            }
            Some(s) if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            Some(s) if !shell => {
                if !pending_paths.is_empty() { pending_paths.push(' '); }
                pending_paths.push_str(s);
                let ends_in_comma = pending_paths.trim_end().chars().last()
                    .is_some_and(|c| matches!(c, ',' | '、' | '，'));
                if !ends_in_comma {
                    sources.extend(parse_paths(&pending_paths)?);
                    pending_paths.clear();
                }
            }
            None if !shell && pending_paths.is_empty() => sources.push(PathBuf::from(arg)),
            None if !shell => return Err("cannot combine non-Unicode paths with a comma-separated list".into()),
            _ => return Err("interactive shell accepts only --config".into()),
        }
    }
    if !pending_paths.is_empty() { sources.extend(parse_paths(&pending_paths)?); }
    if !shell && sources.is_empty() { return Err("missing input path".into()); }
    if !shell && mode.is_none() { return Err("specify --in-place or --copy".into()); }
    Ok(Some(Args { sources, mode, config, shell }))
}

fn execute() -> Result<ExitCode, String> {
    let Some(args) = parse_args(std::env::args_os().skip(1))? else { return Ok(ExitCode::SUCCESS); };
    if args.shell {
        run_repl(args.config.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    let executable = std::env::current_exe()
        .map_err(|e| format!("cannot locate executable: {e}"))?;
    let config = load_config(&executable, args.config.as_deref())?;
    let result = run_batch(&args.sources, args.mode.unwrap(), &config)?;
    println!("{}", result.report());
    Ok(if result.is_success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn main() -> ExitCode {
    match execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_modes_and_shell() {
        let args = parse_args(["a.png,b.jpg".into(), "--copy".into()]).unwrap().unwrap();
        assert_eq!(args.sources, [PathBuf::from("a.png"), PathBuf::from("b.jpg")]);
        assert_eq!(args.mode, Some(Mode::Copy));
        assert!(parse_args(["a.png".into()]).is_err());
        assert!(parse_args(["a.png".into(), "--copy".into(), "--in-place".into()]).is_err());
        let spaced = parse_args(["日本語.png,".into(), "次.webp".into(), "--copy".into()]).unwrap().unwrap();
        assert_eq!(spaced.sources, [PathBuf::from("日本語.png"), PathBuf::from("次.webp")]);
        assert!(parse_args(["shell".into()]).unwrap().unwrap().shell);
        assert!(parse_args(Vec::<OsString>::new()).unwrap().unwrap().shell);
    }
    #[test]
    fn ini_uses_executable_dir() {
        use std::fs;
        let folder = std::env::temp_dir().join(format!("metaclean-main-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("metaclean.ini"), "[keep]\ncreator=\"LocalName\"\n").unwrap();
        assert_eq!(load_config(&folder.join("metaclean.exe"), None).unwrap().creator.replacement(), Some("LocalName"));
        fs::remove_dir_all(folder).unwrap();
    }
}
