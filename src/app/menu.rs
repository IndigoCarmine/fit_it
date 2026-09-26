//! Menu bar, status bar, keyboard shortcuts and dropped files.

use super::FitApp;
use super::i18n::{self, Lang, t};
use super::widgets;
use egui::Ui;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// A menu entry that closes the menu when clicked.
fn item(ui: &mut Ui, enabled: bool, text: &str, hover: Option<&str>) -> bool {
    let mut resp = ui.add_enabled(enabled, egui::Button::new(text));
    if let Some(h) = hover {
        resp = resp.on_hover_text(h);
    }
    let clicked = resp.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl FitApp {
    pub(super) fn handle_input(&mut self, ctx: &egui::Context) {
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

    pub(super) fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("File", "ファイル"), |ui| self.file_menu(ui));
            ui.menu_button(t("Models", "モデル"), |ui| self.models_menu(ui));
            ui.menu_button(t("Data", "データ"), |ui| {
                if item(
                    ui,
                    true,
                    t("Cross-section (value at x)…", "断面 (x での値)…"),
                    None,
                ) {
                    let tag = self.current().map(|i| self.project.datasets[i].tag.clone());
                    self.rt.slice.open_for(tag);
                }
            });
            ui.menu_button(t("View", "表示"), |ui| self.view_menu(ui));
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
                if item(ui, true, t("About", "このアプリについて"), None) {
                    self.rt.windows.about = true;
                }
            });
            ui.add_space(16.0);
            egui::widgets::global_theme_preference_buttons(ui);
        });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let has_data = self.current().is_some();
        if item(
            ui,
            true,
            t("Open data…  (Ctrl+O)", "データを開く…  (Ctrl+O)"),
            None,
        ) {
            self.open_data_dialog();
        }
        if item(ui, true, t("Open project…", "プロジェクトを開く…"), None) {
            self.open_project_dialog();
        }
        if item(
            ui,
            true,
            t("Save project  (Ctrl+S)", "プロジェクトを保存  (Ctrl+S)"),
            None,
        ) {
            self.save_project(false);
        }
        if item(
            ui,
            true,
            t("Save project as…", "名前を付けてプロジェクトを保存…"),
            None,
        ) {
            self.save_project(true);
        }
        ui.separator();
        if item(
            ui,
            has_data,
            t(
                "Export data + fit curves (CSV)…",
                "データとフィット曲線を書き出し (CSV)…",
            ),
            None,
        ) {
            self.export_curves();
        }
        if item(
            ui,
            !self.rt.report.is_empty(),
            t("Export fit report…", "フィットレポートを書き出し…"),
            None,
        ) {
            self.export_report();
        }
        if item(
            ui,
            has_data,
            t(
                "Export global-fit data side by side (CSV)…",
                "グローバルフィットの全データを並べて書き出し (CSV)…",
            ),
            Some(t(
                "All datasets checked for the global fit, one column group each: x, y, model, residual, components",
                "グローバルフィット対象の全データセットを列方向に並べます: x, y, モデル, 残差, 成分",
            )),
        ) {
            self.export_global_csv();
        }
        if item(
            ui,
            has_data,
            t("Export PDF report…", "PDF レポートを書き出し…"),
            Some(t(
                "Plots of every fitted dataset with residuals, parameter tables and the fit report",
                "各データセットのフィット曲線と残差のプロット、パラメータ表、フィットレポート",
            )),
        ) {
            self.export_pdf();
        }
        ui.separator();
        if item(ui, true, t("New project", "新規プロジェクト"), None) {
            self.new_project();
        }
        if !cfg!(target_arch = "wasm32") && ui.button(t("Quit", "終了")).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn models_menu(&mut self, ui: &mut Ui) {
        if item(ui, true, t("Reload models", "モデルを再読み込み"), None) {
            let ctx = ui.ctx().clone();
            self.reload_models(&ctx);
        }
        if item(
            ui,
            true,
            t("Plugins & presets…", "プラグインとプリセット…"),
            None,
        ) {
            self.rt.windows.plugins = true;
        }
        ui.separator();
        if item(
            ui,
            true,
            t("New formula model…", "数式モデルを新規作成…"),
            None,
        ) {
            self.rt.windows.open_formula_editor();
        }
        if item(
            ui,
            true,
            t(
                "New Python / C model from template…",
                "テンプレートから Python / C モデルを作成…",
            ),
            None,
        ) {
            self.rt.windows.new_model = true;
        }
        if item(
            ui,
            true,
            t("Open plugin folder", "プラグインフォルダを開く"),
            None,
        ) {
            self.open_plugin_dir();
        }
    }

    fn view_menu(&mut self, ui: &mut Ui) {
        let v = &mut self.view;
        ui.checkbox(&mut v.log_x, t("Log x", "x 対数"));
        ui.checkbox(&mut v.log_y, t("Log y", "y 対数"));
        ui.separator();
        ui.checkbox(&mut v.show_components, t("Show components", "成分を表示"));
        ui.checkbox(&mut v.show_residuals, t("Show residuals", "残差を表示"));
        ui.checkbox(&mut v.error_bars, t("Show error bars", "誤差棒を表示"));
        ui.checkbox(
            &mut v.overlay,
            t(
                "Overlay datasets in global fit",
                "グローバルフィット対象を重ねて表示",
            ),
        );
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
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
