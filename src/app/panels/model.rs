//! The model builder: components of the selected dataset and how they combine.

use crate::app::FitApp;
use crate::app::i18n::t;
use crate::app::widgets::text_commit;
use crate::model::composite::CompiledComposite;
use crate::model::is_identifier;
use crate::plugin::Entry;
use egui::{RichText, Ui};

/// Tooltip of a model in the "Add component" menu: description, parameters, source.
fn entry_hover(e: &Entry) -> String {
    let info = e.model.info();
    let params: Vec<&str> = info.params.iter().map(|p| p.name.as_str()).collect();
    format!(
        "{}\n\n{}: {}\n{} · {} ({})",
        info.description,
        t("parameters", "パラメータ"),
        params.join(", "),
        e.folder,
        e.file
            .file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default(),
        e.kind.label()
    )
}

impl FitApp {
    pub(in crate::app) fn model_panel(&mut self, ui: &mut Ui) {
        ui.heading(t("Model", "モデル"));
        let Some(sel) = self.current() else {
            ui.label(
                RichText::new(t(
                    "Load data to build a model.",
                    "データを読み込むとモデルを作れます。",
                ))
                .weak(),
            );
            return;
        };
        let registry = self.rt.registry.clone();

        ui.horizontal(|ui| {
            ui.menu_button(t("➕ Add component", "➕ 成分を追加"), |ui| {
                if registry.entries.is_empty() {
                    ui.label(t("No models loaded", "モデルが読み込まれていません"));
                }
                for (cat, entries) in registry.by_category() {
                    ui.menu_button(cat, |ui| {
                        for e in entries {
                            let info = e.model.info();
                            if ui.button(&info.name).on_hover_text(entry_hover(e)).clicked() {
                                ui.close();
                                self.add_component(&info.name.clone());
                            }
                        }
                    });
                }
                ui.separator();
                if ui.button(t("New formula model…", "数式モデルを新規作成…")).clicked() {
                    ui.close();
                    self.rt.windows.open_formula_editor();
                }
            });
            if ui
                .add_enabled(
                    self.project.datasets.len() > 1,
                    egui::Button::new(t("Copy to all", "全体にコピー")),
                )
                .on_hover_text(t(
                    "Give every dataset checked for the global fit this model and these parameter settings",
                    "グローバルフィット対象のすべてのデータセットに、このモデルとパラメータ設定をコピー",
                ))
                .clicked()
            {
                let lookup = self.lookup();
                self.project.copy_model_to_all(sel, &lookup);
                self.set_status(t(
                    "Copied the model to all datasets (starting values guessed from each dataset)",
                    "モデルを全データセットにコピーしました (初期値は各データから推定)",
                ));
            }
        });

        let mut remove = None;
        let mut guess = None;
        let mut rename = None;
        let mut swap = None;
        let comps = self.project.datasets[sel].spec.components.clone();
        egui::Grid::new(("components", sel))
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for (i, c) in comps.iter().enumerate() {
                    if let Some(n) = text_commit(ui, ("comp", sel, i), &c.name, 48.0) {
                        rename = Some((i, n));
                    }
                    let known = registry.get(&c.model).is_some();
                    let label = if known {
                        RichText::new(&c.model)
                    } else {
                        RichText::new(t(
                            format!("{} (missing)", c.model),
                            format!("{} (見つかりません)", c.model),
                        ))
                        .color(ui.visuals().error_fg_color)
                    };
                    ui.label(label);
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(t("Guess", "推定"))
                            .on_hover_text(t(
                                "Estimate from the data minus the other components",
                                "データから他の成分を引いたものから推定",
                            ))
                            .clicked()
                        {
                            guess = Some(i);
                        }
                        if i > 0 && ui.small_button("⏶").clicked() {
                            swap = Some(i);
                        }
                    });
                    if ui.small_button("🗙").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        let lookup = self.lookup();
        if let Some((i, n)) = rename {
            let taken = comps.iter().any(|c| c.name == n);
            if is_identifier(&n) && n != "x" && !taken {
                self.project.rename_component(sel, i, &n);
            } else {
                self.set_error(t(
                    format!("`{n}` cannot be used as a component name"),
                    format!("`{n}` は成分名に使えません"),
                ));
            }
        }
        if let Some(i) = swap {
            self.project.datasets[sel].spec.components.swap(i - 1, i);
            self.project.sync_params(&lookup);
        }
        if let Some(i) = remove {
            self.project.datasets[sel].spec.components.remove(i);
            self.project.sync_params(&lookup);
        }
        if let Some(i) = guess
            && let Err(e) = self.project.guess_component(sel, i, &lookup)
        {
            self.set_error(e);
        }

        let d = &mut self.project.datasets[sel];
        ui.horizontal(|ui| {
            ui.label(t("Formula", "結合式")).on_hover_text(t(
                "How components combine, e.g. (g1 + g2) * e1 + bg.\nEmpty means the sum of all components.\nx and functions like exp(), sqrt() are allowed.",
                "成分の組み合わせ方 (例 (g1 + g2) * e1 + bg)。\n空欄なら全成分の和。\nx や exp(), sqrt() などの関数も使えます。",
            ));
            let hint = d.spec.effective_formula();
            ui.add(egui::TextEdit::singleline(&mut d.spec.formula).hint_text(hint).desired_width(f32::INFINITY));
        });
        if !d.spec.components.is_empty()
            && let Err(e) = CompiledComposite::build(&d.spec, &lookup)
        {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }
}
