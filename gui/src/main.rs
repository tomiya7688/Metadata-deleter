#![forbid(unsafe_code)]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;
use metaclean::config::load_config;
use metaclean::ops::Mode;
use metaclean::shell::{parse_paths, run_batch};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

#[derive(Default)]
struct CleanGui {
    paths: String,
    config_path: String,
    in_place: bool,
    running: bool,
    result: String,
    receiver: Option<Receiver<String>>,
}

impl CleanGui {
    fn add_path(&mut self, path: PathBuf) {
        // A comma in a path must not split it into two inputs.
        let raw = path.to_string_lossy().replace('"', "\"\"");
        if !self.paths.trim().is_empty() { self.paths.push_str(",\n"); }
        if raw.chars().any(|c| matches!(c, ',' | '、' | '，')) { self.paths.push('"'); }
        self.paths.push_str(&raw);
        if raw.chars().any(|c| matches!(c, ',' | '、' | '，')) { self.paths.push('"'); }
    }

    fn run(&mut self) {
        if self.running { return; }
        let paths = self.paths.clone();
        let config = self.config_path.trim().to_owned();
        let mode = if self.in_place { Mode::InPlace } else { Mode::Copy };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.running = true;
        self.result = "Processing...".to_owned();
        std::thread::spawn(move || {
            let result = (|| -> Result<String, String> {
                let paths = parse_paths(&paths)?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let path = if config.is_empty() { None } else { Some(PathBuf::from(config)) };
                let policy = load_config(&exe, path.as_deref())?;
                let report = run_batch(&paths, mode, &policy)?;
                Ok(report.report())
            })();
            let _ = sender.send(match result {
                Ok(report) => report,
                Err(err) => format!("Error: {err}"),
            });
        });
    }
}

impl eframe::App for CleanGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(receiver) = self.receiver.as_ref() {
            if let Ok(message) = receiver.try_recv() {
                self.result = message;
                self.running = false;
                self.receiver = None;
            }
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.iter()
            .filter_map(|f| f.path.clone()).collect::<Vec<_>>());
        for path in dropped { self.add_path(path); }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Metadata deleter");
            ui.label("Files or folders (comma-separated). Drag and drop paths here.");
            ui.add(egui::TextEdit::multiline(&mut self.paths)
                .desired_rows(7).desired_width(f32::INFINITY)
                .hint_text("C:\\Photos\\写真.png, C:\\Photos\\旅行\\\n\"C:\\Photos\\one,two.jpg\""));
            ui.horizontal(|ui| {
                ui.label("Operation:");
                ui.radio_value(&mut self.in_place, false, "Create cleaned copies (recommended)");
                ui.radio_value(&mut self.in_place, true, "Overwrite originals");
            });
            ui.horizontal(|ui| {
                ui.label("INI override (optional):");
                ui.add(egui::TextEdit::singleline(&mut self.config_path)
                    .desired_width(420.0).hint_text("Uses metaclean.ini beside this executable"));
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.running, egui::Button::new("Run")).clicked() { self.run(); }
                if ui.add_enabled(!self.running, egui::Button::new("Clear paths")).clicked() { self.paths.clear(); }
            });
            ui.separator();
            ui.label("Results");
            ui.add(egui::TextEdit::multiline(&mut self.result)
                .desired_rows(10).desired_width(f32::INFINITY).interactive(false));
            ui.small("Fully local. Same parser and processing engine as metaclean shell. No shell commands are executed.");
        });
        if self.running { ctx.request_repaint_after(Duration::from_millis(100)); }
    }
}


/// Load installed Japanese fonts at runtime, without distributing font files.
fn load_japanese_font(ctx: &egui::Context) {
    #[cfg(windows)]
    let candidates = ["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc", "msyh.ttc"]
        .iter().map(|name| {
            std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
                .join("Fonts").join(name)
        }).collect::<Vec<_>>();
    #[cfg(not(windows))]
    let candidates = vec![
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc")
    ];
    for candidate in candidates {
        if let Ok(bytes) = std::fs::read(&candidate) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("system-cjk".to_string(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().insert(0, "system-cjk".to_string());
            }
            ctx.set_fonts(fonts);
            break;
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([780.0, 590.0]),
        ..Default::default()
    };
    eframe::run_native("Metadata deleter", options, Box::new(|cc| { load_japanese_font(&cc.egui_ctx); Ok(Box::<CleanGui>::default()) }))
}
