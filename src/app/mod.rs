//! The egui front end.
//!
//! Layout: datasets and model builder on the left, parameters and fit controls
//! on the right, plots in the middle. Model loading and fitting run on
//! background threads; the UI polls them each frame.
//!
//! `FitApp` is split by concern: `files` (open/save), `exports`, `models`
//! (registry and plugin folder), `fitting`, `menu` (menu and status bars),
//! `panels` (side panels) and `windows` (floating windows).

mod clock;
mod exports;
mod files;
mod fitting;
mod i18n;
mod menu;
mod models;
mod panels;
mod plot;
mod slice_window;
mod widgets;
mod windows;

use crate::model::ModelRef;
use crate::plugin::{self, Registry};
use crate::project::Project;
use i18n::{Lang, t};
use plot::{PlotState, ViewOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub plugin_dir: PathBuf,
    /// Interpreter for `.py` models.
    pub python: String,
    pub language: Lang,
}

impl Default for Settings {
    fn default() -> Self {
        let python = std::env::var("FIT_IT_PYTHON")
            .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into());
        // First launch: follow a Japanese locale where the environment says so.
        let ja = ["LANG", "LC_ALL", "LC_MESSAGES"]
            .iter()
            .any(|k| std::env::var(k).is_ok_and(|v| v.starts_with("ja")));
        Self {
            plugin_dir: plugin::default_plugin_dir(),
            python,
            language: if ja { Lang::Ja } else { Lang::En },
        }
    }
}

#[derive(Default)]
struct Status {
    text: String,
    error: bool,
}

/// State that is rebuilt every launch rather than persisted.
#[derive(Default)]
struct Runtime {
    registry: Arc<Registry>,
    registry_gen: u64,
    loading: Option<mpsc::Receiver<Registry>>,
    fit: Option<fitting::FitTask>,
    /// Parameters (by dataset tag) as they were before the last fit, for
    /// "Undo fit". Only parameters: edits made since (constants, derived
    /// quantities, ...) are kept.
    undo: Option<Vec<(String, Vec<crate::fit::Param>)>>,
    report: String,
    status: Status,
    plot: PlotState,
    windows: windows::WindowState,
    slice: slice_window::SliceWindow,
    /// Model to add to the selected dataset once the next reload finishes.
    pending_add: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FitApp {
    project: Project,
    selected: usize,
    project_path: Option<PathBuf>,
    settings: Settings,
    view: ViewOptions,
    #[serde(skip)]
    rt: Runtime,
}

impl FitApp {
    /// Restores the previous session, then opens `files` (data, projects or model
    /// files, as if dropped on the window).
    pub fn new(cc: &eframe::CreationContext<'_>, files: &[PathBuf]) -> Self {
        let mut app: FitApp = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        i18n::set_lang(app.settings.language);
        i18n::install_cjk_font(&cc.egui_ctx);
        for f in files {
            app.open_path(f);
        }
        app.reload_models(&cc.egui_ctx);
        app
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.rt.status = Status {
            text: text.into(),
            error: false,
        };
    }

    fn set_error(&mut self, text: impl Into<String>) {
        self.rt.status = Status {
            text: text.into(),
            error: true,
        };
    }

    fn status_saved(&mut self, path: &Path) {
        self.set_status(t(
            format!("Saved {}", path.display()),
            format!("{} を保存しました", path.display()),
        ));
    }

    fn lookup(&self) -> impl Fn(&str) -> Option<ModelRef> + use<> {
        let reg = self.rt.registry.clone();
        move |n: &str| reg.get(n)
    }

    /// Bring every dataset's parameters in line with its model (see
    /// [`Project::sync_params`]).
    fn sync_params(&mut self) {
        let lookup = self.lookup();
        self.project.sync_params(&lookup);
    }

    fn current(&self) -> Option<usize> {
        (self.selected < self.project.datasets.len()).then_some(self.selected)
    }

    fn fitting(&self) -> bool {
        self.rt.fit.is_some()
    }
}

impl eframe::App for FitApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_registry();
        self.poll_fit(&ctx);
        self.handle_input(&ctx);
        if std::mem::take(&mut self.rt.windows.reload_requested) {
            self.reload_models(&ctx);
        }
        if self.selected >= self.project.datasets.len() {
            self.selected = self.project.datasets.len().saturating_sub(1);
        }

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));

        let busy = self.fitting();
        egui::Panel::left("left_panel")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        self.datasets_panel(ui);
                        ui.add_space(8.0);
                        self.model_panel(ui);
                    });
                });
            });
        egui::Panel::right("right_panel")
            .resizable(true)
            .default_size(560.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| self.params_panel(ui));
                    ui.add_space(8.0);
                    self.fit_panel(ui, &ctx);
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.project.datasets.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(t(
                        "Drop data files here, or File > Open data…\n\nCSV / TSV / whitespace columns and JASCO exports; comment lines are skipped.",
                        "ここにデータファイルをドロップするか、ファイル > データを開く…\n\nCSV / TSV / 空白区切りの列、JASCO エクスポートに対応。コメント行は無視されます。",
                    ));
                });
                return;
            }
            let key = plot::fingerprint(&self.project, self.selected, &self.view, self.rt.registry_gen);
            let lookup = self.lookup();
            self.rt.plot.update(&self.project, &lookup, self.selected, &self.view, key);
            self.rt.plot.show(ui, &self.view);
        });

        self.show_windows(&ctx);
    }
}
