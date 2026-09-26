//! Floating windows: plugin manager, new-model templates, formula editor, about.

use super::FitApp;
use super::i18n::t;
use super::widgets::num_field;
use crate::model::expression::{ExprModel, ExprModelSpec, ExprParamSpec};
use crate::plugin::{self, Template};
use egui::RichText;

pub struct FormulaEditor {
    spec: ExprModelSpec,
    add_to_model: bool,
}

pub struct WindowState {
    pub plugins: bool,
    pub new_model: bool,
    pub about: bool,
    pub reload_requested: bool,
    new_name: String,
    template: Template,
    formula: Option<FormulaEditor>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            plugins: false,
            new_model: false,
            about: false,
            reload_requested: false,
            new_name: "MyModel".into(),
            template: Template::Python,
            formula: None,
        }
    }
}

impl WindowState {
    pub fn open_formula_editor(&mut self) {
        self.formula = Some(FormulaEditor {
            spec: ExprModelSpec {
                name: "MyFormula".into(),
                category: "Custom".into(),
                formula: "a * exp(-x / tau) + c".into(),
                ..Default::default()
            },
            add_to_model: true,
        });
    }
}

impl FitApp {
    pub(super) fn show_windows(&mut self, ctx: &egui::Context) {
        self.plugins_window(ctx);
        self.new_model_window(ctx);
        self.formula_window(ctx);
        self.slice_window(ctx);
        let mut open = self.rt.windows.about;
        egui::Window::new(t("About", "このアプリについて"))
            .id(egui::Id::new("about_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading(crate::APP_NAME);
                ui.label(format!("{} {}", t("Version", "バージョン"), env!("CARGO_PKG_VERSION")));
                ui.separator();
                ui.label(t(
                    env!("CARGO_PKG_DESCRIPTION"),
                    "GUI カーブフィッティング: プリセットと自作モデル (Python・C・数式) を組み合わせ、1 つまたは複数のデータセットをフィット",
                ));
                ui.label(t(
                    "Fitting: bounded Levenberg–Marquardt (lmfit-style bounds, constraints and reports).",
                    "フィット: 範囲付き Levenberg–Marquardt 法 (lmfit 方式の範囲・拘束・レポート)。",
                ));
                ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
            });
        self.rt.windows.about = open;
    }

    fn plugins_window(&mut self, ctx: &egui::Context) {
        let mut open = self.rt.windows.plugins;
        let registry = self.rt.registry.clone();
        egui::Window::new(t("Plugins & presets", "プラグインとプリセット"))
            .id(egui::Id::new("plugins_window"))
            .open(&mut open).default_width(640.0).show(ctx, |ui| {
            ui.label(
                t(
                    "Presets and plugins are the same kind of file; they only live in different folders. \
                     A plugin with the same model name replaces the preset.",
                    "プリセットとプラグインは同じ形式のファイルで、置き場所のフォルダが違うだけです。\
                     同じモデル名のプラグインはプリセットを置き換えます。",
                ),
            );
            ui.add_space(4.0);
            egui::Grid::new("folders").num_columns(3).show(ui, |ui| {
                ui.label(t("Presets", "プリセット"));
                let presets = plugin::find_presets_dir();
                ui.label(presets.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| t("(not found)", "(見つかりません)").into()));
                if let Some(p) = presets
                    && ui.small_button(t("Open", "開く")).clicked()
                {
                    plugin::open_in_os(&p);
                }
                ui.end_row();
                ui.label(t("Plugins", "プラグイン"));
                ui.label(self.settings.plugin_dir.display().to_string());
                ui.horizontal(|ui| {
                    if ui.small_button(t("Open", "開く")).clicked() {
                        self.open_plugin_dir();
                    }
                    if ui.small_button(t("Change…", "変更…")).clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_directory(&self.settings.plugin_dir).pick_folder()
                    {
                        self.settings.plugin_dir = dir;
                        self.rt.windows.reload_requested = true;
                    }
                });
                ui.end_row();
                ui.label("Python");
                ui.add(egui::TextEdit::singleline(&mut self.settings.python).desired_width(260.0))
                    .on_hover_text(t(
                        "Interpreter for .py models (needs numpy). A change takes effect after restarting fit_it.",
                        ".py モデル用のインタプリタ (numpy が必要)。変更は fit_it の再起動後に反映されます。",
                    ));
                ui.label("");
                ui.end_row();
                ui.label("");
                ui.label(RichText::new(&registry.python_status).weak());
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(t("⟳ Reload models", "⟳ モデルを再読み込み")).clicked() {
                    self.rt.windows.reload_requested = true;
                }
                if ui.button(t("New model from template…", "テンプレートから新規作成…")).clicked() {
                    self.rt.windows.new_model = true;
                }
                if ui.button(t("New formula model…", "数式モデルを新規作成…")).clicked() {
                    self.rt.windows.open_formula_editor();
                }
            });

            if !registry.issues.is_empty() {
                ui.separator();
                ui.label(RichText::new(t("Problems", "問題")).strong());
                egui::ScrollArea::vertical().id_salt("issues").max_height(200.0).show(ui, |ui| {
                    for i in &registry.issues {
                        let color = if i.warning { ui.visuals().warn_fg_color } else { ui.visuals().error_fg_color };
                        ui.colored_label(color, i.file.display().to_string());
                        ui.add(egui::Label::new(RichText::new(&i.message).monospace().size(11.0)).wrap());
                    }
                });
            }

            ui.separator();
            ui.label(RichText::new(t(
                format!("{} models", registry.entries.len()),
                format!("{} 個のモデル", registry.entries.len()),
            )).strong());
            egui::ScrollArea::vertical().id_salt("models").max_height(320.0).show(ui, |ui| {
                egui::Grid::new("model_list").striped(true).num_columns(5).show(ui, |ui| {
                    for h in [
                        t("model", "モデル"),
                        t("category", "カテゴリ"),
                        t("source", "種類"),
                        t("folder", "フォルダ"),
                        t("file", "ファイル"),
                    ] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for (_, entries) in registry.by_category() {
                        for e in entries {
                            let info = e.model.info();
                            ui.label(&info.name).on_hover_text(&info.description);
                            ui.label(&info.category);
                            ui.label(e.kind.label());
                            ui.label(&e.folder);
                            let file = e.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                            if ui.link(file).on_hover_text(t("Open in the default editor", "既定のエディタで開く")).clicked() {
                                plugin::open_in_os(&e.file);
                            }
                            ui.end_row();
                        }
                    }
                });
            });
        });
        self.rt.windows.plugins = open;
    }

    fn new_model_window(&mut self, ctx: &egui::Context) {
        let mut open = self.rt.windows.new_model;
        let mut created = None;
        egui::Window::new(t("New model from template", "テンプレートから新規作成"))
            .id(egui::Id::new("new_model_window"))
            .open(&mut open).collapsible(false).show(ctx, |ui| {
            let w = &mut self.rt.windows;
            ui.horizontal(|ui| {
                ui.label(t("Model name", "モデル名"));
                ui.text_edit_singleline(&mut w.new_name);
            });
            for t in Template::ALL {
                ui.radio_value(&mut w.template, t, t.label());
            }
            ui.label(
                RichText::new(t(
                    "The file is created in the plugin folder and opened in your editor. Save it, then Reload models.",
                    "プラグインフォルダにファイルを作成してエディタで開きます。保存したら「モデルを再読み込み」してください。",
                ))
                    .weak(),
            );
            if ui.button(t("Create", "作成")).clicked() {
                created = Some(plugin::write_template(&self.settings.plugin_dir, w.template, &w.new_name));
            }
        });
        match created {
            Some(Ok(path)) => {
                plugin::open_in_os(&path);
                self.rt.windows.reload_requested = true;
                self.set_status(t(
                    format!("Created {}", path.display()),
                    format!("{} を作成しました", path.display()),
                ));
                open = false;
            }
            Some(Err(e)) => self.set_error(e),
            None => {}
        }
        self.rt.windows.new_model = open;
    }

    fn formula_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.rt.windows.formula else {
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new(t("Formula model", "数式モデル"))
            .id(egui::Id::new("formula_window"))
            .open(&mut open).default_width(480.0).show(ctx, |ui| {
            egui::Grid::new("formula_meta").num_columns(2).show(ui, |ui| {
                ui.label(t("Name", "名前"));
                ui.text_edit_singleline(&mut ed.spec.name);
                ui.end_row();
                ui.label(t("Category", "カテゴリ"));
                ui.text_edit_singleline(&mut ed.spec.category);
                ui.end_row();
                ui.label(t("Description", "説明"));
                ui.text_edit_singleline(&mut ed.spec.description);
                ui.end_row();
                ui.label("y =");
                ui.add(egui::TextEdit::singleline(&mut ed.spec.formula).desired_width(320.0).font(egui::TextStyle::Monospace));
                ui.end_row();
            });
            ui.label(
                RichText::new(t(
                    "Every name except x is a parameter. + - * / ^, exp ln log10 sqrt abs sin cos tan erf pow min max, pi",
                    "x 以外の名前はすべてパラメータ。+ - * / ^、exp ln log10 sqrt abs sin cos tan erf pow min max、pi",
                ))
                    .weak()
                    .size(11.0),
            );

            match ed.spec.detected_params() {
                Ok(names) => {
                    // Keep one row per detected parameter, preserving edits.
                    let mut rows = Vec::new();
                    for n in &names {
                        rows.push(ed.spec.params.iter().find(|p| &p.name == n).cloned().unwrap_or(ExprParamSpec {
                            name: n.clone(),
                            default: Some(1.0),
                            ..Default::default()
                        }));
                    }
                    ed.spec.params = rows;
                    egui::Grid::new("formula_params").striped(true).num_columns(4).show(ui, |ui| {
                        for h in [t("parameter", "パラメータ"), t("default", "既定値"), t("min", "最小"), t("max", "最大")] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for (i, p) in ed.spec.params.iter_mut().enumerate() {
                            ui.label(&p.name);
                            let mut d = p.default.unwrap_or(1.0);
                            num_field(ui, ("fdef", i), &mut d, 70.0, 1.0, "1");
                            p.default = Some(d);
                            let mut lo = p.min.unwrap_or(f64::NEG_INFINITY);
                            num_field(ui, ("fmin", i), &mut lo, 70.0, f64::NEG_INFINITY, "-inf");
                            p.min = lo.is_finite().then_some(lo);
                            let mut hi = p.max.unwrap_or(f64::INFINITY);
                            num_field(ui, ("fmax", i), &mut hi, 70.0, f64::INFINITY, "inf");
                            p.max = hi.is_finite().then_some(hi);
                            ui.end_row();
                        }
                    });
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            let valid = ExprModel::new(&ed.spec);
            if let Err(e) = &valid {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.checkbox(&mut ed.add_to_model, t("Add to the current model after saving", "保存後に現在のモデルへ追加"));
            if ui.add_enabled(valid.is_ok(), egui::Button::new(t("Save to plugin folder", "プラグインフォルダに保存"))).clicked() {
                save = true;
            }
        });
        if save {
            let ed = self.rt.windows.formula.as_ref().unwrap();
            let file_name = format!(
                "{}.fexpr",
                ed.spec
                    .name
                    .to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric() && c != '_', "_")
            );
            let text = format!(
                "# fit_it formula model (created in the app)\n{}",
                ed.spec.to_toml()
            );
            let add = ed.add_to_model.then(|| ed.spec.name.clone());
            if self.write_plugin_file(&file_name, &text) {
                self.rt.pending_add = add;
                open = false;
            }
        }
        if !open {
            self.rt.windows.formula = None;
        }
    }
}
