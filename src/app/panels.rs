//! The side panels: datasets, model builder, parameter table, fit controls.

use super::FitApp;
use super::i18n::t;
use super::widgets::{dataset_color, fmt_num, num_field, text_commit};
use crate::data::{Dataset, Weighting};
use crate::model::composite::CompiledComposite;
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

enum ParamAction {
    Share(String),
    Unshare(String),
    CopyValue(String),
}

impl FitApp {
    pub(super) fn datasets_panel(&mut self, ui: &mut Ui) {
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
        let Some(sel) = self.current() else { return };

        ui.separator();
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

        egui::CollapsingHeader::new(t(
            format!("Constants ({})", d.data.constants.len()),
            format!("定数 ({})", d.data.constants.len()),
        ))
            .id_salt(("constants", sel))
            .default_open(!d.data.constants.is_empty())
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
                    for (name, v) in d.data.constants.iter_mut() {
                        ui.label(name);
                        num_field(ui, ("const", sel, name.as_str()), v, 90.0, 0.0, "0");
                        if ui.small_button("🗙").clicked() {
                            remove = Some(name.clone());
                        }
                        ui.end_row();
                    }
                });
                if let Some(n) = remove {
                    d.data.constants.remove(&n);
                }
                ui.horizontal(|ui| {
                    let id = ui.make_persistent_id(("new_const", sel));
                    let mut name: String = ui.data(|m| m.get_temp(id)).unwrap_or_default();
                    ui.add(egui::TextEdit::singleline(&mut name).hint_text(t("name", "名前")).desired_width(90.0));
                    let ok = is_identifier(&name) && !d.data.constants.contains_key(&name);
                    if ui.add_enabled(ok, egui::Button::new(t("Add", "追加"))).clicked() {
                        d.data.constants.insert(std::mem::take(&mut name), 0.0);
                    }
                    ui.data_mut(|m| m.insert_temp(id, name));
                });
            });

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

    pub(super) fn model_panel(&mut self, ui: &mut Ui) {
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
                            let params: Vec<&str> =
                                info.params.iter().map(|p| p.name.as_str()).collect();
                            let hover = format!(
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
                            );
                            if ui.button(&info.name).on_hover_text(hover).clicked() {
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

    pub(super) fn params_panel(&mut self, ui: &mut Ui) {
        ui.heading(t("Parameters", "パラメータ"));
        let Some(sel) = self.current() else { return };
        let lookup = self.lookup();
        let defs = CompiledComposite::build(&self.project.datasets[sel].spec, &lookup)
            .map(|c| c.params().to_vec())
            .unwrap_or_default();
        if self.project.datasets[sel].params.is_empty() {
            ui.label(
                RichText::new(t(
                    "Add components to the model to get parameters.",
                    "モデルに成分を追加するとパラメータが表示されます。",
                ))
                .weak(),
            );
            return;
        }
        let multi = self.project.datasets.len() > 1;
        let tag = self.project.datasets[sel].tag.clone();
        let shared: Vec<usize> = self.project.datasets[sel]
            .params
            .iter()
            .map(|p| self.project.shared_count(sel, &p.name))
            .collect();
        let derived = &self.rt.plot.values;
        let mut actions = Vec::new();
        let error_color = ui.visuals().error_fg_color;

        egui::Grid::new(("params", sel))
            .striped(true)
            .num_columns(8)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                for h in [
                    t("name", "名前"),
                    t("value", "値"),
                    t("± error", "± 誤差"),
                    t("vary", "可変"),
                    t("min", "最小"),
                    t("max", "最大"),
                    t("constraint", "拘束式"),
                    "",
                ] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, p) in self.project.datasets[sel].params.iter_mut().enumerate() {
                    let def = defs.iter().find(|d| d.name == p.name);
                    let mut hover = p.name.clone();
                    if let Some(d) = def {
                        if !d.unit.is_empty() {
                            hover.push_str(&format!(" [{}]", d.unit));
                        }
                        if !d.description.is_empty() {
                            hover.push_str(&format!("\n{}", d.description));
                        }
                        hover.push_str(&format!(
                            "\n{} {}",
                            t("default", "既定値"),
                            fmt_num(d.default)
                        ));
                    }
                    hover.push_str(&format!(
                        "\n{}: {tag}.{}",
                        t("global name", "全体名"),
                        p.name
                    ));
                    ui.label(&p.name).on_hover_text(hover);

                    let constrained = !p.expr.trim().is_empty();
                    if constrained {
                        let v = derived
                            .get(&format!("{tag}.{}", p.name))
                            .copied()
                            .unwrap_or(p.value);
                        ui.label(RichText::new(fmt_num(v)).italics())
                            .on_hover_text(t("computed from the constraint", "拘束式から計算"));
                    } else {
                        let speed = if p.value != 0.0 {
                            p.value.abs() * 0.003
                        } else if p.min.is_finite() && p.max.is_finite() {
                            (p.max - p.min) * 0.001
                        } else {
                            0.001
                        };
                        let mut drag = egui::DragValue::new(&mut p.value)
                            .speed(speed)
                            .custom_formatter(|v, _| fmt_num(v));
                        if p.min < p.max {
                            drag = drag.range(p.min..=p.max);
                        }
                        ui.add_sized([90.0, 18.0], drag);
                    }
                    match p.stderr {
                        Some(e) => {
                            let pct = if p.value != 0.0 {
                                format!(" ({:.1}%)", 100.0 * e / p.value.abs())
                            } else {
                                String::new()
                            };
                            ui.label(format!("{}{pct}", fmt_num(e)))
                        }
                        None => ui.label(""),
                    };
                    ui.add_enabled(!constrained, egui::Checkbox::without_text(&mut p.vary));
                    num_field(
                        ui,
                        ("min", sel, i),
                        &mut p.min,
                        64.0,
                        f64::NEG_INFINITY,
                        "-inf",
                    );
                    num_field(ui, ("max", sel, i), &mut p.max, 64.0, f64::INFINITY, "inf");
                    let mut edit = egui::TextEdit::singleline(&mut p.expr)
                        .desired_width(110.0)
                        .hint_text(t("e.g. 2*g1_sigma", "例 2*g1_sigma"));
                    if constrained && !derived.contains_key(&format!("{tag}.{}", p.name)) {
                        edit = edit.text_color(error_color);
                    }
                    ui.add(edit);
                    ui.menu_button(if shared[i] > 0 { "↔" } else { "⚙" }, |ui| {
                        if ui
                            .add_enabled(
                                multi,
                                egui::Button::new(t(
                                    format!("Share {} with all datasets", p.name),
                                    format!("{} を全データセットで共有", p.name),
                                )),
                            )
                            .on_hover_text(t(
                                format!("Other datasets get the constraint {tag}.{}", p.name),
                                format!("他のデータセットに拘束式 {tag}.{} を設定します", p.name),
                            ))
                            .clicked()
                        {
                            actions.push(ParamAction::Share(p.name.clone()));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                shared[i] > 0,
                                egui::Button::new(t(
                                    format!("Stop sharing ({} linked)", shared[i]),
                                    format!("共有を解除 ({} 件リンク中)", shared[i]),
                                )),
                            )
                            .clicked()
                        {
                            actions.push(ParamAction::Unshare(p.name.clone()));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                multi,
                                egui::Button::new(t(
                                    "Copy value to all datasets",
                                    "値を全データセットにコピー",
                                )),
                            )
                            .clicked()
                        {
                            actions.push(ParamAction::CopyValue(p.name.clone()));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(t("Reset to default", "既定値に戻す")).clicked() {
                            if let Some(d) = def {
                                p.value = d.default;
                                p.min = d.min;
                                p.max = d.max;
                            }
                            p.expr.clear();
                            p.vary = true;
                            ui.close();
                        }
                    });
                    ui.end_row();
                }
            });

        for a in actions {
            match a {
                ParamAction::Share(n) => {
                    self.project.share_param(sel, &n);
                    self.set_status(t(
                        format!("{tag}.{n} is now shared by all datasets that have {n}"),
                        format!("{tag}.{n} を、{n} を持つ全データセットで共有しました"),
                    ));
                }
                ParamAction::Unshare(n) => self.project.unshare_param(sel, &n),
                ParamAction::CopyValue(n) => {
                    let v = self.project.datasets[sel].param(&n).map(|p| p.value);
                    if let Some(v) = v {
                        for d in &mut self.project.datasets {
                            if let Some(p) = d.params.iter_mut().find(|p| p.name == n) {
                                p.value = v;
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn fit_panel(&mut self, ui: &mut Ui, ctx: &egui::Context) {
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
