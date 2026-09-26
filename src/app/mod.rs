//! The egui front end.
//!
//! Layout: datasets and model builder on the left, parameters and fit controls
//! on the right, plots in the middle. Model loading and fitting run on
//! background threads; the UI polls them each frame.

mod i18n;
mod panels;
mod plot;
mod slice_window;
mod widgets;
mod windows;

use crate::data::Dataset;
use crate::fit::{self, FitOutcome, Progress};
use crate::model::ModelRef;
use crate::plugin::{self, Folder, LoadOptions, Registry};
use crate::project::{DerivedSpec, PROJECT_EXTENSION, Project};
use i18n::{Lang, t};
use plot::{PlotState, ViewOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

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

struct FitTask {
    handle: JoinHandle<Result<FitOutcome, String>>,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    label: String,
    started: Instant,
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
    fit: Option<FitTask>,
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

    fn lookup(&self) -> impl Fn(&str) -> Option<ModelRef> + use<> {
        let reg = self.rt.registry.clone();
        move |n: &str| reg.get(n)
    }

    fn folders(&self) -> Vec<Folder> {
        let mut v = Vec::new();
        if let Some(p) = plugin::find_presets_dir() {
            v.push(Folder {
                label: "Presets".into(),
                path: p,
            });
        }
        v.push(Folder {
            label: "Plugins".into(),
            path: self.settings.plugin_dir.clone(),
        });
        v
    }

    fn fitting(&self) -> bool {
        self.rt.fit.is_some()
    }

    // ---- model registry ---------------------------------------------------

    fn reload_models(&mut self, ctx: &egui::Context) {
        if let Err(e) = plugin::prepare_plugin_dir(&self.settings.plugin_dir) {
            self.set_error(e);
        }
        let folders = self.folders();
        let opts = LoadOptions {
            python: self.settings.python.clone(),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Registry::load(&folders, &opts));
            ctx.request_repaint();
        });
        self.rt.loading = Some(rx);
        self.set_status(t("Loading models…", "モデルを読み込み中…"));
    }

    fn poll_registry(&mut self) {
        let Some(rx) = &self.rt.loading else { return };
        let Ok(reg) = rx.try_recv() else { return };
        self.rt.loading = None;
        let errors = reg.issues.iter().filter(|i| !i.warning).count();
        let n = reg.entries.len();
        self.rt.registry = Arc::new(reg);
        self.rt.registry_gen += 1;
        let lookup = self.lookup();
        self.project.sync_params(&lookup);
        if errors > 0 {
            self.set_error(t(
                format!("Loaded {n} models; {errors} file(s) failed — see Models > Plugins & presets"),
                format!("{n} 個のモデルを読み込みました。{errors} 個のファイルでエラー — モデル > プラグインとプリセット を参照"),
            ));
        } else {
            self.set_status(t(
                format!("Loaded {n} models"),
                format!("{n} 個のモデルを読み込みました"),
            ));
        }
        if let Some(name) = self.rt.pending_add.take() {
            self.add_component(&name);
        }
    }

    fn add_component(&mut self, model: &str) {
        let Some(ds) = self.current() else {
            self.set_error(t("Load a dataset first", "先にデータを読み込んでください"));
            return;
        };
        let name = self.project.add_component(ds, model);
        let lookup = self.lookup();
        self.project.sync_params(&lookup);
        let idx = self.project.datasets[ds].spec.components.len() - 1;
        match self.project.guess_component(ds, idx, &lookup) {
            Ok(()) => self.set_status(t(
                format!("Added {name} ({model}) with guessed initial values"),
                format!("{name} ({model}) を追加し、初期値を推定しました"),
            )),
            Err(_) => self.set_status(t(
                format!("Added {name} ({model})"),
                format!("{name} ({model}) を追加しました"),
            )),
        }
    }

    // ---- datasets / files ---------------------------------------------------

    fn current(&self) -> Option<usize> {
        (self.selected < self.project.datasets.len()).then_some(self.selected)
    }

    fn open_data_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            // "All files" first: JASCO exports are often renamed (`.txt.re`, `.txta`).
            .add_filter(t("All files", "すべてのファイル"), &["*"])
            .add_filter(
                t("Data", "データ"),
                &["csv", "tsv", "txt", "dat", "xy", "asc", "prn"],
            )
            .pick_files()
        {
            for p in paths {
                self.open_path(&p);
            }
        }
    }

    fn load_data(&mut self, path: &Path) {
        match Dataset::load(path) {
            Ok(d) => {
                let n = d.table.rows();
                let had_model = self
                    .current()
                    .map(|i| self.project.datasets[i].spec.clone());
                let i = self.project.add_dataset(d);
                // New data usually needs the same model as what is on screen.
                if let Some(spec) = had_model.filter(|s| !s.components.is_empty()) {
                    self.project.datasets[i].spec = spec;
                    let src = self.selected;
                    self.project.datasets[i].params = self.project.datasets[src].params.clone();
                }
                self.selected = i;
                let lookup = self.lookup();
                self.project.sync_params(&lookup);
                self.set_status(t(
                    format!("Loaded {} ({n} rows)", path.display()),
                    format!("{} を読み込みました ({n} 行)", path.display()),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Open whatever was dropped or picked: a project, a model file or data.
    fn open_path(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(&format!(".{PROJECT_EXTENSION}")) {
            self.open_project(path);
        } else if ["py", "c", "fexpr", "dll", "so", "dylib"].contains(&ext.as_str()) {
            let dest = self
                .settings
                .plugin_dir
                .join(path.file_name().unwrap_or_default());
            match plugin::prepare_plugin_dir(&self.settings.plugin_dir)
                .and_then(|_| std::fs::copy(path, &dest).map_err(|e| e.to_string()))
            {
                Ok(_) => {
                    self.rt.windows.reload_requested = true;
                    self.set_status(t(
                        format!("Copied {} into the plugin folder", dest.display()),
                        format!("{} をプラグインフォルダにコピーしました", dest.display()),
                    ));
                }
                Err(e) => self.set_error(e),
            }
        } else {
            self.load_data(path);
        }
    }

    fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.project = p;
                self.selected = 0;
                self.project_path = Some(path.to_path_buf());
                let lookup = self.lookup();
                self.project.sync_params(&lookup);
                self.rt.undo = None;
                self.set_status(t(
                    format!("Opened {}", path.display()),
                    format!("{} を開きました", path.display()),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }

    fn save_project(&mut self, save_as: bool) {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .add_filter(
                    t("fit_it project", "fit_it プロジェクト"),
                    &[PROJECT_EXTENSION],
                )
                .set_file_name(format!("project.{PROJECT_EXTENSION}"))
                .save_file(),
        };
        let Some(mut path) = path else { return };
        if !path.to_string_lossy().ends_with(PROJECT_EXTENSION) {
            path = PathBuf::from(format!(
                "{}.{PROJECT_EXTENSION}",
                path.with_extension("").display()
            ));
        }
        match self.project.save(&path) {
            Ok(()) => {
                self.set_status(t(
                    format!("Saved {}", path.display()),
                    format!("{} を保存しました", path.display()),
                ));
                self.project_path = Some(path);
            }
            Err(e) => self.set_error(e),
        }
    }

    fn export_curves(&mut self) {
        let Some(ds) = self.current() else { return };
        let d = &self.project.datasets[ds];
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{}_fit.csv", d.tag))
            .save_file()
        else {
            return;
        };
        let lookup = self.lookup();
        let result = (|| -> Result<(), String> {
            let problem = self.project.problem(&lookup, &|_| false)?;
            let k = problem
                .datasets()
                .iter()
                .position(|j| j.tag == d.tag)
                .ok_or(t(
                    "this dataset has no model",
                    "このデータセットにはモデルがありません",
                ))?;
            let model = &problem.datasets()[k].model;
            let values = problem.current_values();
            let pv = problem.dataset_values(k, &values);
            let (x, y) = (d.data.x(), d.data.y());
            let n = x.len().min(y.len());
            let mut f = vec![0.0; n];
            model.eval(&x[..n], pv, &mut f)?;
            let comps = model.eval_components(&x[..n], pv)?;
            let sigma = d.data.sigma();
            let mut out = String::from("x,y");
            if sigma.is_some() {
                out.push_str(",sigma");
            }
            out.push_str(",model,residual,in_fit_range");
            for c in model.component_names() {
                out.push_str(&format!(",{c}"));
            }
            out.push('\n');
            for i in 0..n {
                out.push_str(&format!("{},{}", x[i], y[i]));
                if let Some(s) = sigma {
                    out.push_str(&format!(",{}", s.get(i).copied().unwrap_or(f64::NAN)));
                }
                out.push_str(&format!(
                    ",{},{},{}",
                    f[i],
                    y[i] - f[i],
                    u8::from(d.data.in_range(x[i]))
                ));
                for c in &comps {
                    out.push_str(&format!(",{}", c[i]));
                }
                out.push('\n');
            }
            std::fs::write(&path, out).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => self.set_status(t(
                format!("Exported {}", path.display()),
                format!("{} に書き出しました", path.display()),
            )),
            Err(e) => self.set_error(e),
        }
    }

    /// Tags of the datasets an export covers (checked for the global fit, else the selected one).
    fn export_tags(&self, set: &[usize]) -> String {
        set.iter()
            .map(|&i| self.project.datasets[i].tag.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Every dataset checked for the global fit, side by side in one CSV.
    fn export_global_csv(&mut self) {
        let set = crate::export::export_set(&self.project, self.selected);
        if set.is_empty() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("global_fit.csv")
            .save_file()
        else {
            return;
        };
        let lookup = self.lookup();
        let result = crate::export::dataset_curves(&self.project, &lookup, &set, 0, false)
            .map(|c| crate::export::side_by_side_csv(&c))
            .and_then(|csv| std::fs::write(&path, csv).map_err(|e| e.to_string()));
        let tags = self.export_tags(&set);
        match result {
            Ok(()) => self.set_status(t(
                format!("Exported {tags} side by side to {}", path.display()),
                format!("{tags} を並べて {} に書き出しました", path.display()),
            )),
            Err(e) => self.set_error(e),
        }
    }

    /// PDF report: overview plot, one page per dataset (fit, residuals,
    /// parameters) and the full fit report.
    fn export_pdf(&mut self) {
        let set = crate::export::export_set(&self.project, self.selected);
        if set.is_empty() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name("fit_report.pdf")
            .save_file()
        else {
            return;
        };
        let lookup = self.lookup();
        let curves =
            match crate::export::dataset_curves(&self.project, &lookup, &set, 600, self.view.log_x)
            {
                Ok(c) => c,
                Err(e) => {
                    self.set_error(e);
                    return;
                }
            };
        let mut info = Vec::new();
        if let Some(first) = self.rt.report.lines().next() {
            info.push(first.to_string());
        } else {
            info.push("Not fitted yet: curves use the current parameter values".into());
        }
        info.push(format!("Datasets: {}", self.export_tags(&set)));
        if let Some(p) = &self.project_path {
            info.push(format!("Project: {}", p.display()));
        }
        info.push(format!(
            "Created: {} (fit_it {})",
            utc_datetime(),
            env!("CARGO_PKG_VERSION")
        ));
        let input = crate::report_pdf::ReportInput {
            title: "fit_it report".into(),
            info,
            curves: &curves,
            colors: set.clone(),
            params: set
                .iter()
                .map(|&i| self.project.datasets[i].params.clone())
                .collect(),
            report_text: &self.rt.report,
            log_x: self.view.log_x,
            log_y: self.view.log_y,
        };
        let bytes = crate::report_pdf::render(&input);
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.set_status(t(
                    format!("Saved {}", path.display()),
                    format!("{} を保存しました", path.display()),
                ));
                plugin::open_in_os(&path);
            }
            Err(e) => self.set_error(e.to_string()),
        }
    }

    fn export_report(&mut self) {
        if self.rt.report.is_empty() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t("Text", "テキスト"), &["txt"])
            .set_file_name("fit_report.txt")
            .save_file()
        {
            match std::fs::write(&path, &self.rt.report) {
                Ok(()) => self.set_status(t(
                    format!("Saved {}", path.display()),
                    format!("{} を保存しました", path.display()),
                )),
                Err(e) => self.set_error(e.to_string()),
            }
        }
    }

    // ---- fitting ----------------------------------------------------------------

    fn start_fit(&mut self, global: bool, ctx: &egui::Context) {
        if self.fitting() {
            return;
        }
        let Some(sel) = self.current() else { return };
        let lookup = self.lookup();
        let include: Vec<bool> = self.project.datasets.iter().map(|d| d.include).collect();
        let active = move |i: usize| if global { include[i] } else { i == sel };
        let problem = match self.project.problem(&lookup, &active) {
            Ok(p) => p,
            Err(e) => {
                self.set_error(e);
                return;
            }
        };
        let label = if global {
            t("Global fit", "グローバルフィット").to_string()
        } else {
            t(
                format!("Fit {}", self.project.datasets[sel].tag),
                format!("{} のフィット", self.project.datasets[sel].tag),
            )
        };
        self.rt.undo = Some(
            self.project
                .datasets
                .iter()
                .map(|d| (d.tag.clone(), d.params.clone()))
                .collect(),
        );
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let opts = self.project.options;
        let specs = DerivedSpec::pairs(&self.project.derived);
        let tag = self.project.datasets[sel].tag.clone();
        let (p2, c2, ctx2) = (progress.clone(), cancel.clone(), ctx.clone());
        let handle = std::thread::spawn(move || {
            let r = fit::fit(&problem, &opts, Some(&p2), Some(&c2)).map(|mut out| {
                out.derived = fit::derive(&problem, &out, &specs, Some(&tag));
                out
            });
            ctx2.request_repaint();
            r
        });
        self.rt.fit = Some(FitTask {
            handle,
            progress,
            cancel,
            label,
            started: Instant::now(),
        });
        self.set_status(t("Fitting…", "フィット中…"));
    }

    fn poll_fit(&mut self, ctx: &egui::Context) {
        let Some(task) = &self.rt.fit else { return };
        if !task.handle.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let task = self.rt.fit.take().unwrap();
        let secs = task.started.elapsed().as_secs_f64();
        match task.handle.join() {
            Ok(Ok(out)) => {
                self.project.clear_stderr();
                self.project.apply_outcome(&out);
                self.rt.report = format!("{} — {}\n{}", task.label, utc_clock(), out.report());
                let chi = widgets::fmt_num(out.redchi);
                let msg = if out.success {
                    t(
                        format!(
                            "{}: converged — reduced χ² = {chi}, {} evaluations, {secs:.2} s",
                            task.label, out.nfev
                        ),
                        format!(
                            "{}: 収束 — reduced χ² = {chi}、評価 {} 回、{secs:.2} 秒",
                            task.label, out.nfev
                        ),
                    )
                } else {
                    t(
                        format!(
                            "{}: not converged — reduced χ² = {chi}, {} evaluations, {secs:.2} s",
                            task.label, out.nfev
                        ),
                        format!(
                            "{}: 未収束 — reduced χ² = {chi}、評価 {} 回、{secs:.2} 秒",
                            task.label, out.nfev
                        ),
                    )
                };
                if out.success {
                    self.set_status(msg);
                } else {
                    self.set_error(format!("{msg} ({})", out.message));
                }
            }
            Ok(Err(e)) => self.set_error(format!("{}: {e}", task.label)),
            Err(_) => self.set_error(t(
                format!("{}: the fit crashed", task.label),
                format!("{}: フィットが異常終了しました", task.label),
            )),
        }
    }

    fn undo_fit(&mut self) {
        if let Some(saved) = self.rt.undo.take() {
            for (tag, params) in saved {
                if let Some(d) = self.project.datasets.iter_mut().find(|d| d.tag == tag) {
                    d.params = params;
                }
            }
            self.set_status(t(
                "Restored parameters from before the fit",
                "フィット前のパラメータに戻しました",
            ));
        }
    }

    // ---- frame ------------------------------------------------------------------

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for p in dropped {
            self.open_path(&p);
        }
        let (open, save, fit) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
            )
        });
        if open {
            self.open_data_dialog();
        }
        if save {
            self.save_project(false);
        }
        if fit {
            self.start_fit(false, ctx);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("File", "ファイル"), |ui| {
                if ui
                    .button(t("Open data…  (Ctrl+O)", "データを開く…  (Ctrl+O)"))
                    .clicked()
                {
                    ui.close();
                    self.open_data_dialog();
                }
                if ui
                    .button(t("Open project…", "プロジェクトを開く…"))
                    .clicked()
                {
                    ui.close();
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(t("fit_it project", "fit_it プロジェクト"), &["json"])
                        .pick_file()
                    {
                        self.open_project(&p);
                    }
                }
                if ui
                    .button(t("Save project  (Ctrl+S)", "プロジェクトを保存  (Ctrl+S)"))
                    .clicked()
                {
                    ui.close();
                    self.save_project(false);
                }
                if ui
                    .button(t("Save project as…", "名前を付けてプロジェクトを保存…"))
                    .clicked()
                {
                    ui.close();
                    self.save_project(true);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.current().is_some(),
                        egui::Button::new(t(
                            "Export data + fit curves (CSV)…",
                            "データとフィット曲線を書き出し (CSV)…",
                        )),
                    )
                    .clicked()
                {
                    ui.close();
                    self.export_curves();
                }
                if ui
                    .add_enabled(
                        !self.rt.report.is_empty(),
                        egui::Button::new(t("Export fit report…", "フィットレポートを書き出し…")),
                    )
                    .clicked()
                {
                    ui.close();
                    self.export_report();
                }
                if ui
                    .add_enabled(
                        self.current().is_some(),
                        egui::Button::new(t(
                            "Export global-fit data side by side (CSV)…",
                            "グローバルフィットの全データを並べて書き出し (CSV)…",
                        )),
                    )
                    .on_hover_text(t(
                        "All datasets checked for the global fit, one column group each: x, y, model, residual, components",
                        "グローバルフィット対象の全データセットを列方向に並べます: x, y, モデル, 残差, 成分",
                    ))
                    .clicked()
                {
                    ui.close();
                    self.export_global_csv();
                }
                if ui
                    .add_enabled(
                        self.current().is_some(),
                        egui::Button::new(t("Export PDF report…", "PDF レポートを書き出し…")),
                    )
                    .on_hover_text(t(
                        "Plots of every fitted dataset with residuals, parameter tables and the fit report",
                        "各データセットのフィット曲線と残差のプロット、パラメータ表、フィットレポート",
                    ))
                    .clicked()
                {
                    ui.close();
                    self.export_pdf();
                }
                ui.separator();
                if ui.button(t("New project", "新規プロジェクト")).clicked() {
                    ui.close();
                    self.project = Project::default();
                    self.project_path = None;
                    self.rt.report.clear();
                    self.rt.undo = None;
                }
                if !cfg!(target_arch = "wasm32") && ui.button(t("Quit", "終了")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(t("Models", "モデル"), |ui| {
                if ui
                    .button(t("Reload models", "モデルを再読み込み"))
                    .clicked()
                {
                    ui.close();
                    self.reload_models(&ctx);
                }
                if ui
                    .button(t("Plugins & presets…", "プラグインとプリセット…"))
                    .clicked()
                {
                    ui.close();
                    self.rt.windows.plugins = true;
                }
                ui.separator();
                if ui
                    .button(t("New formula model…", "数式モデルを新規作成…"))
                    .clicked()
                {
                    ui.close();
                    self.rt.windows.open_formula_editor();
                }
                if ui
                    .button(t(
                        "New Python / C model from template…",
                        "テンプレートから Python / C モデルを作成…",
                    ))
                    .clicked()
                {
                    ui.close();
                    self.rt.windows.new_model = true;
                }
                if ui
                    .button(t("Open plugin folder", "プラグインフォルダを開く"))
                    .clicked()
                {
                    ui.close();
                    let _ = plugin::prepare_plugin_dir(&self.settings.plugin_dir);
                    plugin::open_in_os(&self.settings.plugin_dir);
                }
            });
            ui.menu_button(t("Data", "データ"), |ui| {
                if ui
                    .button(t("Cross-section (value at x)…", "断面 (x での値)…"))
                    .clicked()
                {
                    ui.close();
                    let tag = self.current().map(|i| self.project.datasets[i].tag.clone());
                    self.rt.slice.open_for(tag);
                }
            });
            ui.menu_button(t("View", "表示"), |ui| {
                ui.checkbox(&mut self.view.log_x, t("Log x", "x 対数"));
                ui.checkbox(&mut self.view.log_y, t("Log y", "y 対数"));
                ui.separator();
                ui.checkbox(
                    &mut self.view.show_components,
                    t("Show components", "成分を表示"),
                );
                ui.checkbox(
                    &mut self.view.show_residuals,
                    t("Show residuals", "残差を表示"),
                );
                ui.checkbox(
                    &mut self.view.error_bars,
                    t("Show error bars", "誤差棒を表示"),
                );
                ui.checkbox(
                    &mut self.view.overlay,
                    t(
                        "Overlay datasets in global fit",
                        "グローバルフィット対象を重ねて表示",
                    ),
                );
            });
            ui.menu_button(t("Language", "言語"), |ui| {
                for lang in Lang::ALL {
                    if ui
                        .radio_value(&mut self.settings.language, lang, lang.label())
                        .clicked()
                    {
                        i18n::set_lang(lang);
                        ui.close();
                    }
                }
            });
            ui.menu_button(t("Help", "ヘルプ"), |ui| {
                if ui.button(t("About", "このアプリについて")).clicked() {
                    ui.close();
                    self.rt.windows.about = true;
                }
            });
            ui.add_space(16.0);
            egui::widgets::global_theme_preference_buttons(ui);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(task) = &self.rt.fit {
                ui.spinner();
                let p = task.progress.lock().map(|p| p.clone()).unwrap_or_default();
                let chi = widgets::fmt_num(p.chisqr);
                ui.label(t(
                    format!(
                        "{}: iteration {}, {} evaluations, χ² = {chi}",
                        task.label, p.iter, p.nfev
                    ),
                    format!(
                        "{}: 反復 {}、評価 {} 回、χ² = {chi}",
                        task.label, p.iter, p.nfev
                    ),
                ));
                if ui.button(t("Cancel", "中止")).clicked() {
                    task.cancel.store(true, Ordering::Relaxed);
                }
            } else if self.rt.loading.is_some() {
                ui.spinner();
                ui.label(t("Loading models…", "モデルを読み込み中…"));
            } else if self.rt.status.error {
                ui.colored_label(ui.visuals().error_fg_color, &self.rt.status.text);
            } else {
                ui.label(&self.rt.status.text);
            }
        });
    }
}

/// Current date and time as `YYYY-MM-DD HH:MM UTC`.
fn utc_datetime() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = s.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let secs = s.rem_euclid(86400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs / 60) % 60
    )
}

/// Current time as `HH:MM:SS UTC`, to tell successive reports apart.
fn utc_clock() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02} UTC",
        (s / 3600) % 24,
        (s / 60) % 60,
        s % 60
    )
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
