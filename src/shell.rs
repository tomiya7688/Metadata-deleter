//! Shared input and batch engine for the command line, interactive shell, and GUI.
//! This is a command dispatcher, not an operating-system shell: no input is executed.
use crate::config::Config;
use crate::ops::{self, Mode, Report};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// Parse comma- and Japanese-comma-separated paths. Double quotes protect
/// commas and leading/trailing whitespace inside one path; "" escapes a quote.
pub fn parse_paths(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => {
                if !quoted && closed {
                    return Err("unexpected quote after a quoted path".into());
                }
                quoted = !quoted;
                if !quoted { closed = true; }
            }
            ',' | '、' | '，' if !quoted => {
                let path = current.trim();
                if path.is_empty() { return Err("empty path in comma-separated list".into()); }
                paths.push(PathBuf::from(path));
                current.clear();
                closed = false;
            }
            ch if closed && !ch.is_whitespace() => return Err("unexpected characters after a quoted path".into()),
            ch => current.push(ch),
        }
    }
    if quoted { return Err("unclosed double quote in path list".into()); }
    let last = current.trim();
    if last.is_empty() { return Err("empty path in comma-separated list".into()); }
    paths.push(PathBuf::from(last));
    Ok(paths)
}

/// Reject duplicate and overlapping inputs before starting a batch. Prevents
/// file+parent-folder requests from processing the same file twice.
fn validate_targets(paths: &[PathBuf], mode: Mode) -> Result<(), String> {
    if paths.is_empty() { return Err("no input paths".into()); }
    let mut canonical: Vec<(PathBuf, bool)> = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symbolic links are not accepted: {}", path.display()));
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(format!("not a regular file or folder: {}", path.display()));
        }
        let absolute = path.canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for (earlier, earlier_dir) in &canonical {
            if &absolute == earlier
                || (metadata.is_dir() && earlier.starts_with(&absolute))
                || (*earlier_dir && absolute.starts_with(earlier))
            {
                return Err(format!("duplicate/overlapping inputs: {}", path.display()));
            }
        }
        if mode == Mode::Copy {
            let output = ops::output_path(path, metadata.is_dir())?;
            if output.exists() {
                return Err(format!("copy destination already exists: {}", output.display()));
            }
        }
        canonical.push((absolute, metadata.is_dir()));
    }
    Ok(())
}

#[derive(Default)]
pub struct BatchResult {
    pub cleaned: usize,
    pub unchanged: usize,
    pub copied: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub outputs: Vec<PathBuf>,
}

impl BatchResult {
    pub fn is_success(&self) -> bool { self.errors.is_empty() }
    pub fn report(&self) -> String {
        let mut lines = Vec::new();
        for path in &self.outputs { lines.push(format!("Output: {}", path.display())); }
        lines.push(format!("Cleaned: {}, unchanged: {}, copied: {}, skipped: {}, errors: {}",
            self.cleaned, self.unchanged, self.copied, self.skipped, self.errors.len()));
        for error in &self.errors { lines.push(format!("error: {error}")); }
        lines.join("\n")
    }
    fn absorb(&mut self, report: Report, output: Option<PathBuf>) {
        self.cleaned += report.cleaned;
        self.unchanged += report.unchanged;
        self.copied += report.copied;
        self.skipped += report.skipped;
        self.errors.extend(report.errors);
        if let Some(path) = output { self.outputs.push(path); }
    }
}

pub fn run_batch(paths: &[PathBuf], mode: Mode, config: &Config) -> Result<BatchResult, String> {
    validate_targets(paths, mode)?;
    let mut result = BatchResult::default();
    for path in paths {
        match ops::run(path, mode, config) {
            Ok((report, output)) => result.absorb(report, output),
            Err(err) => result.errors.push(err),
        }
    }
    Ok(result)
}

pub fn help() -> &'static str {
    "Commands:\n  copy <path1>, <path2>      Create cleaned copies\n  in-place <path1>, <path2>  Clean originals\n  help                       Show this help\n  exit                       Quit\n\nPaths can be files or folders (folders are recursive).\nWrap paths with commas in double quotes. Japanese filenames are supported.\nAll operations use metaclean.ini next to the executable.\n"
}

pub fn run_repl(config_path: Option<&Path>) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Metadata deleter shell\n{}", help()).map_err(|e| e.to_string())?;
    loop {
        write!(stdout, "metaclean> ").and_then(|_| stdout.flush()).map_err(|e| e.to_string())?;
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 { break; }
        let command = line.trim();
        if command.is_empty() { continue; }
        if command.eq_ignore_ascii_case("exit") || command.eq_ignore_ascii_case("quit") { break; }
        if command.eq_ignore_ascii_case("help") { writeln!(stdout, "{}", help()).map_err(|e| e.to_string())?; continue; }
        let (mode, tail) = match command.split_once(char::is_whitespace) {
            Some(("copy", tail)) => (Mode::Copy, tail.trim()),
            Some(("in-place", tail)) => (Mode::InPlace, tail.trim()),
            _ => {
                writeln!(stdout, "Unknown command. Enter help.").map_err(|e| e.to_string())?;
                continue;
            }
        };
        let operation = || -> Result<BatchResult, String> {
            let paths = parse_paths(tail)?;
            // Reload before every command so editing the INI takes effect immediately.
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let config = crate::config::load_config(&executable, config_path)?;
            run_batch(&paths, mode, &config)
        };
        match operation() {
            Ok(result) => writeln!(stdout, "{}", result.report()).map_err(|e| e.to_string())?,
            Err(error) => writeln!(stdout, "error: {error}").map_err(|e| e.to_string())?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn japanese_and_quoted_commas() {
        assert_eq!(parse_paths("写真.png, 画像.webp、\"東京,夜景.jpg\"").unwrap(),
            ["写真.png", "画像.webp", "東京,夜景.jpg"].map(PathBuf::from));
        assert_eq!(parse_paths("\"A\\B\\日本語.png\"").unwrap(), [PathBuf::from("A\\B\\日本語.png")]);
    }
    #[test]
    fn invalid_lists_rejected() {
        for value in ["", "a.png,", ",a.png", "a.png,,b.jpg", "\"a.png", "\"a.png\"xxx"] {
            assert!(parse_paths(value).is_err(), "{value}");
        }
    }
    #[test]
    fn duplicates_rejected() {
        let p = std::env::temp_dir();
        assert!(validate_targets(&[p.clone(), p], Mode::InPlace).is_err());
    }
}
