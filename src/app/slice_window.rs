//! The "Cross-section" window: spectra → fit-ready curves (see `crate::transform`).

use super::FitApp;
use super::i18n::t;
use super::widgets::num_field;
use crate::transform::{SliceOptions, series_columns, slice_across, slice_columns};
use egui::RichText;
use std::collections::BTreeSet;

pub struct SliceWindow {
    pub open: bool,
    /// true: one point per dataset (x = a constant); false: one curve per dataset
    /// from its spectrum columns (x = column header, e.g. temperature).
    across: bool,
    sources: BTreeSet<String>,
    x_constant: String,
    divide: bool,
    divide_by: String,
    baseline: bool,
    base_lo: f64,
    base_hi: f64,
    exclude_sources: bool,
    opts: SliceOptions,
}

impl Default for SliceWindow {
    fn default() -> Self {
        Self {
            open: false,
            across: false,
            sources: BTreeSet::new(),
            x_constant: "conc_uM".into(),
            divide: false,
            divide_by: "conc_uM".into(),
            baseline: true,
            base_lo: 450.0,
            base_hi: f64::INFINITY,
            exclude_sources: true,
            opts: SliceOptions {
                at: 300.0,
                x_label: "T (°C)".into(),
                ..Default::default()
            },
        }
    }
}

impl SliceWindow {
    pub fn open_for(&mut self, tag: Option<String>) {
        self.open = true;
        if self.sources.is_empty()
            && let Some(t) = tag
        {
            self.sources.insert(t);
        }
    }
}

impl FitApp {
    pub(super) fn slice_window(&mut self, ctx: &egui::Context) {
        let mut open = self.rt.slice.open;
        let mut create = false;
        let all_constants: BTreeSet<String> = self
            .project
            .datasets
            .iter()
            .flat_map(|d| d.data.constants.keys().cloned())
            .collect();
        egui::Window::new(t("Cross-section: value at x", "断面: x での値"))
            .id(egui::Id::new("slice_window"))
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                let w = &mut self.rt.slice;
                ui.radio_value(
                    &mut w.across,
                    false,
                    t("Temperature scan: one curve per dataset", "温度スキャン: データセットごとに 1 本の曲線"),
                )
                .on_hover_text(t(
                    "Each spectrum column (JASCO 2-D file) gives one point; x = its header value, e.g. temperature",
                    "各スペクトル列 (JASCO 2 次元ファイル) が 1 点になります。x = 列見出しの値 (例: 温度)",
                ));
                ui.radio_value(
                    &mut w.across,
                    true,
                    t("Titration: one curve from all selected datasets", "滴定: 選択した全データセットから 1 本の曲線"),
                )
                .on_hover_text(t(
                    "Each dataset gives one point; x = one of its constants, e.g. conc_uM",
                    "各データセットが 1 点になります。x = そのデータセットの定数 (例: conc_uM)",
                ));

                ui.separator();
                ui.label(RichText::new(t("Source datasets", "元データセット")).strong());
                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                    for d in &self.project.datasets {
                        let mut on = w.sources.contains(&d.tag);
                        let cols = series_columns(&d.data).len();
                        let hint = if cols > 0 {
                            t(format!("  ({cols} spectra)"), format!("  ({cols} スペクトル)"))
                        } else {
                            String::new()
                        };
                        if ui.checkbox(&mut on, format!("{}  {}{hint}", d.tag, d.data.name)).changed() {
                            if on {
                                w.sources.insert(d.tag.clone());
                            } else {
                                w.sources.remove(&d.tag);
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.small_button(t("all", "すべて")).clicked() {
                        w.sources = self.project.datasets.iter().map(|d| d.tag.clone()).collect();
                    }
                    if ui.small_button(t("none", "なし")).clicked() {
                        w.sources.clear();
                    }
                });

                ui.separator();
                let constant_combo = |ui: &mut egui::Ui, id: &str, value: &mut String| {
                    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).show_ui(ui, |ui| {
                        for c in &all_constants {
                            ui.selectable_value(value, c.clone(), c);
                        }
                    });
                };
                egui::Grid::new("slice_opts").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label(t("Read at x", "読み取る x"));
                    ui.horizontal(|ui| {
                        num_field(ui, "slice_at", &mut w.opts.at, 70.0, 0.0, "nm");
                        ui.label(t("± window", "± 幅"));
                        num_field(ui, "slice_win", &mut w.opts.window, 50.0, 0.0, t("0 = nearest", "0 = 最近傍"));
                    });
                    ui.end_row();

                    ui.checkbox(&mut w.baseline, t("Baseline", "ベースライン"));
                    ui.add_enabled_ui(w.baseline, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(t("subtract mean over", "次の範囲の平均を差し引く"));
                            num_field(ui, "base_lo", &mut w.base_lo, 55.0, f64::NEG_INFINITY, "-inf");
                            ui.label("–");
                            num_field(ui, "base_hi", &mut w.base_hi, 55.0, f64::INFINITY, "inf");
                        });
                    });
                    ui.end_row();

                    ui.checkbox(&mut w.divide, t("Divide by", "次で割る"));
                    ui.add_enabled_ui(w.divide, |ui| constant_combo(ui, "divide_by", &mut w.divide_by));
                    ui.end_row();

                    if w.across {
                        ui.label(t("x = constant", "x = 定数"));
                        constant_combo(ui, "x_constant", &mut w.x_constant);
                        ui.end_row();
                    }

                    ui.label(t("x := x·scale + offset", "x := x·倍率 + オフセット"));
                    ui.horizontal(|ui| {
                        num_field(ui, "x_scale", &mut w.opts.x_scale, 60.0, 1.0, "1");
                        num_field(ui, "x_offset", &mut w.opts.x_offset, 60.0, 0.0, "0");
                    });
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        if ui.small_button(t("°C to K", "°C を K に")).clicked() {
                            w.opts.x_scale = 1.0;
                            w.opts.x_offset = 273.15;
                            w.opts.x_label = "T (K)".into();
                        }
                        if ui.small_button(t("µM to M", "µM を M に")).clicked() {
                            w.opts.x_scale = 1e-6;
                            w.opts.x_offset = 0.0;
                            w.opts.x_label = "c (M)".into();
                        }
                    });
                    ui.end_row();

                    ui.label(t("x label", "x ラベル"));
                    ui.text_edit_singleline(&mut w.opts.x_label);
                    ui.end_row();

                    ui.label("y");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut w.opts.normalize, t("normalise 0…1", "0…1 に正規化"));
                        ui.checkbox(&mut w.opts.invert, t("invert (1 − y)", "反転 (1 − y)"));
                    });
                    ui.end_row();
                });
                ui.checkbox(
                    &mut w.exclude_sources,
                    t("Uncheck the sources from the global fit", "元データセットをグローバルフィットから外す"),
                );
                ui.add_space(4.0);
                if ui
                    .add_enabled(!w.sources.is_empty(), egui::Button::new(RichText::new(t("Create dataset(s)", "データセットを作成")).strong()))
                    .clicked()
                {
                    create = true;
                }
            });
        self.rt.slice.open = open;
        if create {
            self.create_slices();
        }
    }

    fn create_slices(&mut self) {
        let w = &self.rt.slice;
        let mut o = w.opts.clone();
        o.baseline = w.baseline.then_some((w.base_lo, w.base_hi));
        o.divide_by = w.divide.then(|| w.divide_by.clone());
        let sources: Vec<usize> = (0..self.project.datasets.len())
            .filter(|&i| w.sources.contains(&self.project.datasets[i].tag))
            .collect();
        let made = if w.across {
            let refs: Vec<&crate::data::Dataset> = sources
                .iter()
                .map(|&i| &self.project.datasets[i].data)
                .collect();
            slice_across(&refs, &w.x_constant, &o).map(|d| vec![d])
        } else {
            sources
                .iter()
                .map(|&i| slice_columns(&self.project.datasets[i].data, &o))
                .collect::<Result<Vec<_>, _>>()
        };
        match made {
            Ok(list) => {
                if self.rt.slice.exclude_sources {
                    for &i in &sources {
                        self.project.datasets[i].include = false;
                    }
                }
                let n = list.len();
                for d in list {
                    self.selected = self.project.add_dataset(d);
                }
                let lookup = self.lookup();
                self.project.sync_params(&lookup);
                self.set_status(t(
                    format!("Created {n} dataset(s) at x = {}", o.at),
                    format!("x = {} で {n} 個のデータセットを作成しました", o.at),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }
}
