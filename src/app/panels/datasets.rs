//! The datasets list and the selected dataset's columns, weights, fit range
//! and constants.

use crate::app::FitApp;
use crate::app::i18n::t;
use crate::app::widgets::{dataset_color, num_field, text_commit};
use crate::data::{Dataset, Weighting};
use crate::model::is_identifier;
use crate::project::DatasetState;
use egui::{RichText, Ui};

fn weighting_label(w: Weighting) -> &'static str {
    match w {
        Weighting::Sigma => t("σ column", "σ 列"),
        Weighting::Unit => t("none (unit)", "なし (等重み)"),
        Weighting::Poisson => t("Poisson √y", "ポアソン √y"),
        Weighting::Relative => t("relative (σ ∝ y)", "相対 (σ ∝ y)"),
    }
}

impl FitApp {
    pub(in crate::app) fn datasets_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading(t("Datasets", "データセット"));
            if ui
                .button(t("Open…", "開く…"))
                .on_hover_text(t(
                    "Or drop files onto the window",
                    "ウィンドウへのドロップでも開けます",
                ))
                .clicked()
            {
                self.open_data_dialog();
            }
        });
        let mut remove = None;
        for (i, d) in self.project.datasets.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.include, "")
                    .on_hover_text(t("Include in global fit", "グローバルフィットに含める"));
                let text =
                    RichText::new(format!("■ {}  {}", d.tag, d.data.name)).color(dataset_color(i));
                if ui.selectable_label(i == self.selected, text).clicked() {
                    self.selected = i;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("🗙")
                        .on_hover_text(t("Remove dataset", "データセットを削除"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.project.datasets.remove(i);
        }
        if let Some(sel) = self.current() {
            ui.separator();
            self.dataset_settings(ui, sel);
        }
    }

    /// Columns, weights, fit range and constants of dataset `sel`.
    fn dataset_settings(&mut self, ui: &mut Ui, sel: usize) {
        let tags: Vec<String> = self
            .project
            .datasets
            .iter()
            .map(|d| d.tag.clone())
            .collect();
        let mut range_to_all = false;
        let d = &mut self.project.datasets[sel];
        egui::Grid::new("dataset_settings")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label(t("Tag", "タグ")).on_hover_text(t(
                    "Short name used in constraints, e.g. D1.g1_sigma",
                    "拘束式で使う短い名前 (例 D1.g1_sigma)",
                ));
                if let Some(t) = text_commit(ui, ("tag", sel), &d.tag, 80.0)
                    && is_identifier(&t)
                    && !tags.contains(&t)
                {
                    d.tag = t;
                }
                ui.end_row();

                let headers = d.data.table.headers.clone();
                let col_combo = |ui: &mut Ui, label: &str, value: &mut usize| {
                    egui::ComboBox::from_id_salt((label, sel))
                        .selected_text(headers.get(*value).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (i, h) in headers.iter().enumerate() {
                                ui.selectable_value(value, i, h);
                            }
                        });
                };
                ui.label("x");
                col_combo(ui, "x", &mut d.data.x_col);
                ui.end_row();
                ui.label("y");
                col_combo(ui, "y", &mut d.data.y_col);
                ui.end_row();
                ui.label("σ");
                egui::ComboBox::from_id_salt(("sigma", sel))
                    .selected_text(
                        d.data
                            .sigma_col
                            .and_then(|c| headers.get(c).cloned())
                            .unwrap_or_else(|| t("none", "なし").into()),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.data.sigma_col, None, t("none", "なし"));
                        for (i, h) in headers.iter().enumerate() {
                            ui.selectable_value(&mut d.data.sigma_col, Some(i), h);
                        }
                    });
                ui.end_row();
                ui.label(t("Weights", "重み"));
                egui::ComboBox::from_id_salt(("weights", sel))
                    .selected_text(weighting_label(d.data.weighting))
                    .show_ui(ui, |ui| {
                        for w in Weighting::ALL {
                            ui.selectable_value(&mut d.data.weighting, w, weighting_label(w));
                        }
                    });
                ui.end_row();

                ui.label(t("Fit range", "フィット範囲"));
                ui.horizontal(|ui| {
                    let (mut lo, mut hi) =
                        d.data.range.unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
                    let a = num_field(
                        ui,
                        ("lo", sel),
                        &mut lo,
                        64.0,
                        f64::NEG_INFINITY,
                        t("min", "最小"),
                    );
                    ui.label("–");
                    let b = num_field(
                        ui,
                        ("hi", sel),
                        &mut hi,
                        64.0,
                        f64::INFINITY,
                        t("max", "最大"),
                    );
                    if a || b {
                        d.data.range = (lo.is_finite() || hi.is_finite()).then_some((lo, hi));
                    }
                });
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    if ui
                        .small_button(t("= view", "= 表示範囲"))
                        .on_hover_text(t(
                            "Fit only what is visible in the plot",
                            "プロットに表示中の範囲だけをフィット",
                        ))
                        .clicked()
                    {
                        d.data.range = self.rt.plot.view_x;
                    }
                    if ui.small_button(t("all", "全体")).clicked() {
                        d.data.range = None;
                    }
                    if ui
                        .small_button(t("to all datasets", "全データセットへ"))
                        .on_hover_text(t(
                            "Use this fit range for every dataset checked for the global fit",
                            "グローバルフィット対象のすべてのデータセットにこの範囲を適用",
                        ))
                        .clicked()
                    {
                        range_to_all = true;
                    }
                });
                ui.end_row();
            });
        let a = d.data.fit_arrays();
        ui.label(
            RichText::new(t(
                format!("{} of {} points in the fit", a.x.len(), d.data.table.rows()),
                format!("{} / {} 点をフィットに使用", a.x.len(), d.data.table.rows()),
            ))
            .weak(),
        );

        let slice = ui
            .small_button(t("Cross-section…", "断面…"))
            .on_hover_text(t(
                "Value at a wavelength vs temperature (JASCO scan) or vs concentration (titration)",
                "ある波長での値を温度 (JASCO スキャン) または濃度 (滴定) に対してプロット",
            ))
            .clicked();
        let split = d.data.table.columns.len() > 2
            && ui
                .small_button(t("One dataset per y column", "y 列ごとにデータセットに分割"))
                .on_hover_text(t(
                    "Split a multi-column file into datasets that share the x column, for a global fit",
                    "多列ファイルを x 列を共有するデータセットに分割 (グローバルフィット用)",
                ))
                .clicked();

        constants_section(ui, &mut d.data, sel);

        if range_to_all {
            self.project.range_to_all(sel);
            self.set_status(t(
                "Fit range applied to all datasets",
                "フィット範囲を全データセットに適用しました",
            ));
        }
        if split {
            self.split_columns(sel);
        }
        if slice {
            let tag = self.project.datasets[sel].tag.clone();
            self.rt.slice.open_for(Some(tag));
        }
    }

    fn split_columns(&mut self, sel: usize) {
        let src = self.project.datasets[sel].clone();
        let skip = [Some(src.data.x_col), src.data.sigma_col];
        let mut first = true;
        for c in 0..src.data.table.columns.len() {
            if skip.contains(&Some(c)) {
                continue;
            }
            let data = Dataset {
                name: format!("{}:{}", src.data.name, src.data.table.headers[c]),
                y_col: c,
                sigma_col: None,
                ..src.data.clone()
            };
            if first {
                self.project.datasets[sel].data = data;
                first = false;
            } else {
                let tag = self.project.unique_tag();
                self.project.datasets.push(DatasetState {
                    tag,
                    data,
                    include: true,
                    ..src.clone()
                });
            }
        }
        self.set_status(t(
            "Split into one dataset per column",
            "列ごとのデータセットに分割しました",
        ));
    }
}

/// The dataset's named constants (read from the file name, editable).
fn constants_section(ui: &mut Ui, data: &mut Dataset, sel: usize) {
    egui::CollapsingHeader::new(t(
        format!("Constants ({})", data.constants.len()),
        format!("定数 ({})", data.constants.len()),
    ))
        .id_salt(("constants", sel))
        .default_open(!data.constants.is_empty())
        .show(ui, |ui| {
            ui.label(
                RichText::new(t(
                    "Known sample values, usable in constraints (e.g. c_tot = conc_M). Read from the file name.",
                    "既知の試料の値。拘束式で使えます (例 c_tot = conc_M)。ファイル名から読み取ります。",
                ))
                    .weak()
                    .size(11.0),
            );
            let mut remove = None;
            egui::Grid::new(("constants_grid", sel)).num_columns(3).show(ui, |ui| {
                for (name, v) in data.constants.iter_mut() {
                    ui.label(name);
                    num_field(ui, ("const", sel, name.as_str()), v, 90.0, 0.0, "0");
                    if ui.small_button("🗙").clicked() {
                        remove = Some(name.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(n) = remove {
                data.constants.remove(&n);
            }
            ui.horizontal(|ui| {
                let id = ui.make_persistent_id(("new_const", sel));
                let mut name: String = ui.data(|m| m.get_temp(id)).unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut name).hint_text(t("name", "名前")).desired_width(90.0));
                let ok = is_identifier(&name) && !data.constants.contains_key(&name);
                if ui.add_enabled(ok, egui::Button::new(t("Add", "追加"))).clicked() {
                    data.constants.insert(std::mem::take(&mut name), 0.0);
                }
                ui.data_mut(|m| m.insert_temp(id, name));
            });
        });
}
