//! Fit controls and options, derived quantities and the fit report.

use crate::app::FitApp;
use crate::app::i18n::t;
use crate::app::widgets::{fmt_num, num_field};
use egui::{RichText, Ui};

impl FitApp {
    pub(in crate::app) fn fit_panel(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.heading(t("Fit", "フィット"));
        let n_global = self.project.datasets.iter().filter(|d| d.include).count();
        let has_data = self.current().is_some();
        ui.horizontal(|ui| {
            let busy = self.fitting();
            let tag = self.current().map(|i| self.project.datasets[i].tag.clone()).unwrap_or_default();
            if ui
                .add_enabled(
                    !busy && has_data,
                    egui::Button::new(RichText::new(t(format!("▶ Fit {tag}"), format!("▶ {tag} をフィット"))).strong()),
                )
                .on_hover_text(t(
                    "Fit the selected dataset (Ctrl+Enter). Constraints to other datasets use their current values.",
                    "選択中のデータセットをフィット (Ctrl+Enter)。他のデータセットへの拘束は現在値を使います。",
                ))
                .clicked()
            {
                self.start_fit(false, ctx);
            }
            if ui
                .add_enabled(
                    !busy && n_global > 0,
                    egui::Button::new(t(format!("▶▶ Global fit ({n_global})"), format!("▶▶ グローバルフィット ({n_global})"))),
                )
                .on_hover_text(t(
                    "Fit all checked datasets together; shared parameters are fitted jointly",
                    "チェックしたデータセットを同時にフィット。共有パラメータはまとめて最適化されます",
                ))
                .clicked()
            {
                self.start_fit(true, ctx);
            }
            if ui.add_enabled(!busy && self.rt.undo.is_some(), egui::Button::new(t("Undo fit", "フィットを元に戻す"))).clicked() {
                self.undo_fit();
            }
        });
        egui::CollapsingHeader::new(t("Options", "オプション"))
            .id_salt("fit_options_header")
            .show(ui, |ui| {
                let o = &mut self.project.options;
                egui::Grid::new("fit_options")
                    .num_columns(2)
                    .show(ui, |ui| {
                        use crate::fit::FitAlgorithm;
                        let alg_text = |a: FitAlgorithm| match a {
                            FitAlgorithm::LevenbergMarquardt => t("Levenberg–Marquardt (local)", "Levenberg–Marquardt（局所）"),
                            FitAlgorithm::MultiStart => t("Multi-start LM", "マルチスタート LM"),
                            FitAlgorithm::DifferentialEvolution => t("Differential evolution + LM", "差分進化 + LM"),
                            FitAlgorithm::BasinHopping => t("Basin hopping", "ベイスンホッピング"),
                        };
                        ui.label(t("algorithm", "アルゴリズム")).on_hover_text(t(
                            "Global methods search the whole bound range for the lowest χ² (many local minima: multi-peak spectra, oscillations) and finish with LM. Parameters without finite bounds are searched within ± search width × max(|value|, 1).",
                            "大域的手法は境界範囲全体から最小の χ² を探し（局所解が多い場合: 多ピーク、振動など）、最後に LM で仕上げます。有限の境界がないパラメータは ± 探索幅 × max(|値|, 1) の範囲で探索します。",
                        ));
                        egui::ComboBox::from_id_salt("fit_algorithm")
                            .selected_text(alg_text(o.algorithm))
                            .show_ui(ui, |ui| {
                                for a in FitAlgorithm::ALL {
                                    ui.selectable_value(&mut o.algorithm, a, alg_text(a));
                                }
                            });
                        ui.end_row();
                        if o.algorithm.is_global() {
                            ui.label(t("global evaluations", "大域探索の評価回数"));
                            ui.add(egui::DragValue::new(&mut o.global_max_nfev).range(100..=100_000_000));
                            ui.end_row();
                            match o.algorithm {
                                FitAlgorithm::MultiStart => {
                                    ui.label(t("LM starts", "LM 開始点の数"));
                                    ui.add(egui::DragValue::new(&mut o.global_starts).range(1..=100_000));
                                    ui.end_row();
                                }
                                FitAlgorithm::BasinHopping => {
                                    ui.label(t("hops", "ホップ回数"));
                                    ui.add(egui::DragValue::new(&mut o.global_starts).range(1..=100_000));
                                    ui.end_row();
                                }
                                FitAlgorithm::DifferentialEvolution => {
                                    ui.label(t("population (0 = auto)", "個体数（0 = 自動）"));
                                    ui.add(egui::DragValue::new(&mut o.population).range(0..=10_000));
                                    ui.end_row();
                                }
                                FitAlgorithm::LevenbergMarquardt => {}
                            }
                            ui.label(t("search width", "探索幅"));
                            ui.add(egui::DragValue::new(&mut o.search_width).range(0.01..=1e6).speed(0.1));
                            ui.end_row();
                            ui.label(t("seed", "乱数シード"));
                            ui.add(egui::DragValue::new(&mut o.seed));
                            ui.end_row();
                        }
                        ui.label(t("max evaluations", "最大評価回数"));
                        ui.add(egui::DragValue::new(&mut o.max_nfev).range(10..=10_000_000));
                        ui.end_row();
                        ui.label("ftol");
                        num_field(ui, "ftol", &mut o.ftol, 80.0, 1e-10, "1e-10");
                        ui.end_row();
                        ui.label("xtol");
                        num_field(ui, "xtol", &mut o.xtol, 80.0, 1e-10, "1e-10");
                        ui.end_row();
                    });
            });
        self.derived_section(ui);
        if !self.rt.report.is_empty() {
            egui::CollapsingHeader::new(t("Fit report", "フィットレポート"))
                .id_salt("fit_report_header")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button(t("Copy", "コピー")).clicked() {
                            ui.ctx().copy_text(self.rt.report.clone());
                        }
                        if ui.small_button(t("Save…", "保存…")).clicked() {
                            self.export_report();
                        }
                        if ui
                            .small_button("PDF…")
                            .on_hover_text(t(
                                "PDF report with plots of every fitted dataset",
                                "各データセットのプロット付き PDF レポート",
                            ))
                            .clicked()
                        {
                            self.export_pdf();
                        }
                    });
                    let mut text = self.rt.report.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
        }
    }

    fn derived_section(&mut self, ui: &mut Ui) {
        let tag = self
            .current()
            .map(|i| self.project.datasets[i].tag.clone())
            .unwrap_or_default();
        egui::CollapsingHeader::new(t(
            format!("Derived quantities ({})", self.project.derived.len()),
            format!("派生量 ({})", self.project.derived.len()),
        ))
            .id_salt("derived_header")
            .default_open(!self.project.derived.is_empty())
            .show(ui, |ui| {
                ui.label(
                    RichText::new(t(
                        format!(
                            "Expressions of parameters and constants; bare names refer to {tag}, D2.name to another dataset. \
                             Reported with propagated errors after each fit."
                        ),
                        format!(
                            "パラメータと定数の式。名前だけなら {tag}、D2.name で他のデータセットを参照します。\
                             フィットごとに誤差伝播付きでレポートに出力されます。"
                        ),
                    ))
                    .weak()
                    .size(11.0),
                );
                let error_color = ui.visuals().error_fg_color;
                let mut remove = None;
                egui::Grid::new("derived").num_columns(4).striped(true).show(ui, |ui| {
                    for (i, d) in self.project.derived.iter_mut().enumerate() {
                        // add_sized: inside a Grid a TextEdit otherwise shrinks to the
                        // column width measured on the previous frame.
                        ui.add_sized([90.0, 18.0], egui::TextEdit::singleline(&mut d.name).hint_text(t("name", "名前")));
                        ui.add_sized(
                            [230.0, 18.0],
                            egui::TextEdit::singleline(&mut d.expr).hint_text(t("e.g. t1_deltaH - 300 * t1_deltaS", "例 t1_deltaH - 300 * t1_deltaS")),
                        );
                        match self.rt.plot.derived.get(i) {
                            Some(Ok(v)) => ui.label(fmt_num(*v)),
                            Some(Err(e)) => ui.colored_label(error_color, t("error", "エラー")).on_hover_text(e),
                            None => ui.label(""),
                        };
                        if ui.small_button("🗙").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    self.project.derived.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.small_button(t("➕ Add", "➕ 追加")).clicked() {
                        self.project.derived.push(Default::default());
                    }
                    ui.menu_button(t("Presets", "プリセット"), |ui| {
                        ui.label(RichText::new(t("For TempCooperative component t1", "TempCooperative 成分 t1 用")).weak());
                        for (name, expr) in THERMO_PRESETS {
                            if ui.button(format!("{name} = {expr}")).clicked() {
                                self.project.derived.push(crate::project::DerivedSpec {
                                    name: name.to_string(),
                                    expr: expr.to_string(),
                                });
                                ui.close();
                            }
                        }
                    });
                });
            });
    }
}

/// Handy derived quantities for temperature-dependent supramolecular models
/// (component named `t1`), as in the UV analysis scripts: values at 300 K.
const THERMO_PRESETS: [(&str, &str); 4] = [
    ("deltaG_300K", "t1_deltaH - 300 * t1_deltaS"),
    (
        "K_elong_300K",
        "exp(-(t1_deltaH - 300 * t1_deltaS) / (8.314 * 300))",
    ),
    (
        "K_c_300K",
        "exp(-(t1_deltaH - 300 * t1_deltaS) / (8.314 * 300)) * t1_c_tot",
    ),
    ("sigma_300K", "exp(-t1_deltaHnuc / (8.314 * 300))"),
];
