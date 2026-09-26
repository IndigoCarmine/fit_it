//! The parameter table of the selected dataset.

use crate::app::FitApp;
use crate::app::i18n::t;
use crate::app::widgets::{fmt_num, num_field};
use crate::fit::Param;
use crate::model::ParamDef;
use crate::model::composite::CompiledComposite;
use egui::{RichText, Ui};

enum ParamAction {
    Share(String),
    Unshare(String),
    CopyValue(String),
}

/// Tooltip of a parameter name: unit, description, default and global name.
fn param_hover(p: &Param, def: Option<&ParamDef>, tag: &str) -> String {
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
    hover
}

/// Drag speed for a value: relative to its size, else to its bounds.
fn drag_speed(p: &Param) -> f64 {
    if p.value != 0.0 {
        p.value.abs() * 0.003
    } else if p.min.is_finite() && p.max.is_finite() {
        (p.max - p.min) * 0.001
    } else {
        0.001
    }
}

/// `stderr (relative %)`, or nothing before a fit.
fn stderr_text(p: &Param) -> String {
    match p.stderr {
        Some(e) => {
            let pct = if p.value != 0.0 {
                format!(" ({:.1}%)", 100.0 * e / p.value.abs())
            } else {
                String::new()
            };
            format!("{}{pct}", fmt_num(e))
        }
        None => String::new(),
    }
}

/// The ⚙ / ↔ menu of one parameter. "Reset to default" applies directly; the
/// actions that touch other datasets are returned.
fn param_menu(
    ui: &mut Ui,
    p: &mut Param,
    def: Option<&ParamDef>,
    tag: &str,
    multi: bool,
    shared: usize,
) -> Option<ParamAction> {
    let mut action = None;
    ui.menu_button(if shared > 0 { "↔" } else { "⚙" }, |ui| {
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
            action = Some(ParamAction::Share(p.name.clone()));
            ui.close();
        }
        if ui
            .add_enabled(
                shared > 0,
                egui::Button::new(t(
                    format!("Stop sharing ({shared} linked)"),
                    format!("共有を解除 ({shared} 件リンク中)"),
                )),
            )
            .clicked()
        {
            action = Some(ParamAction::Unshare(p.name.clone()));
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
            action = Some(ParamAction::CopyValue(p.name.clone()));
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
    action
}

impl FitApp {
    pub(in crate::app) fn params_panel(&mut self, ui: &mut Ui) {
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
                    ui.label(&p.name).on_hover_text(param_hover(p, def, &tag));

                    let global = format!("{tag}.{}", p.name);
                    let constrained = !p.expr.trim().is_empty();
                    if constrained {
                        let v = derived.get(&global).copied().unwrap_or(p.value);
                        ui.label(RichText::new(fmt_num(v)).italics())
                            .on_hover_text(t("computed from the constraint", "拘束式から計算"));
                    } else {
                        let speed = drag_speed(p);
                        let mut drag = egui::DragValue::new(&mut p.value)
                            .speed(speed)
                            .custom_formatter(|v, _| fmt_num(v));
                        if p.min < p.max {
                            drag = drag.range(p.min..=p.max);
                        }
                        ui.add_sized([90.0, 18.0], drag);
                    }
                    ui.label(stderr_text(p));
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
                    if constrained && !derived.contains_key(&global) {
                        edit = edit.text_color(error_color);
                    }
                    ui.add(edit);
                    actions.extend(param_menu(ui, p, def, &tag, multi, shared[i]));
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
                ParamAction::CopyValue(n) => self.project.copy_value_to_all(sel, &n),
            }
        }
    }
}
