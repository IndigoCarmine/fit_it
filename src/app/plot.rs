//! Data / model / residual plots.
//!
//! Curves are recomputed only when something that affects them changes (see
//! [`fingerprint`]), so dragging a parameter re-evaluates the model once per
//! change rather than every frame — which matters for Python models.

use super::i18n::t;
use super::widgets::dataset_color;
use crate::model::ModelRef;
use crate::project::Project;
use egui::Color32;
use egui_plot::{
    AxisHints, GridMark, HLine, Legend, Line, LineStyle, Plot, Points, Span, log_grid_spacer,
};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ViewOptions {
    pub log_x: bool,
    pub log_y: bool,
    pub show_components: bool,
    pub show_residuals: bool,
    pub error_bars: bool,
    /// Show every dataset included in the global fit, not just the selected one.
    pub overlay: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            log_x: false,
            log_y: false,
            show_components: true,
            show_residuals: true,
            error_bars: true,
            overlay: false,
        }
    }
}

struct Curves {
    name: String,
    color: Color32,
    inside: Vec<[f64; 2]>,
    outside: Vec<[f64; 2]>,
    /// (x, y - σ, y + σ), already transformed.
    errors: Vec<(f64, f64, f64)>,
    model: Vec<[f64; 2]>,
    components: Vec<(String, Vec<[f64; 2]>)>,
    residuals: Vec<[f64; 2]>,
    /// Residuals are divided by σ (not just y - f).
    weighted: bool,
    range: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct PlotState {
    key: u64,
    curves: Vec<Curves>,
    pub error: Option<String>,
    /// Current value of every parameter, constraints applied (`tag.name`).
    pub values: HashMap<String, f64>,
    /// Current value of each derived quantity (same order as the project's list).
    pub derived: Vec<Result<f64, String>>,
    /// Visible x range of the main plot, in data units.
    pub view_x: Option<(f64, f64)>,
}

fn hash_f64(h: &mut DefaultHasher, v: f64) {
    v.to_bits().hash(h);
}

pub fn fingerprint(
    project: &Project,
    selected: usize,
    view: &ViewOptions,
    registry_gen: u64,
) -> u64 {
    let mut h = DefaultHasher::new();
    (
        registry_gen,
        selected,
        view.log_x,
        view.log_y,
        view.overlay,
        view.show_components,
        view.error_bars,
    )
        .hash(&mut h);
    for d in &project.derived {
        (&d.name, &d.expr).hash(&mut h);
    }
    for d in &project.datasets {
        (&d.tag, d.include, &d.spec.formula).hash(&mut h);
        for c in &d.spec.components {
            (&c.name, &c.model).hash(&mut h);
        }
        for p in &d.params {
            (&p.name, &p.expr, p.vary).hash(&mut h);
            hash_f64(&mut h, p.value);
        }
        for (name, v) in &d.data.constants {
            name.hash(&mut h);
            hash_f64(&mut h, *v);
        }
        let data = &d.data;
        (
            &data.name,
            &data.path,
            data.x_col,
            data.y_col,
            data.sigma_col,
            data.table.rows(),
        )
            .hash(&mut h);
        (data.weighting as u8).hash(&mut h);
        if let Some((a, b)) = data.range {
            hash_f64(&mut h, a);
            hash_f64(&mut h, b);
        }
    }
    h.finish()
}

fn tf(v: f64, log: bool) -> Option<f64> {
    if !v.is_finite() {
        None
    } else if log {
        (v > 0.0).then(|| v.log10())
    } else {
        Some(v)
    }
}

fn pt(x: f64, y: f64, view: &ViewOptions) -> Option<[f64; 2]> {
    Some([tf(x, view.log_x)?, tf(y, view.log_y)?])
}

impl PlotState {
    pub fn update(
        &mut self,
        project: &Project,
        lookup: &dyn Fn(&str) -> Option<ModelRef>,
        selected: usize,
        view: &ViewOptions,
        key: u64,
    ) {
        if key == self.key {
            return;
        }
        self.key = key;
        self.curves.clear();
        self.values.clear();
        self.derived.clear();
        self.error = None;

        // A problem with nothing active evaluates every constraint without fitting.
        let problem = match project.problem(lookup, &|_| false) {
            Ok(p) => Some(p),
            Err(e) => {
                self.error = Some(e);
                None
            }
        };
        let values = problem
            .as_ref()
            .map(|p| p.current_values())
            .unwrap_or_default();
        if let Some(p) = &problem {
            let tag = project.datasets.get(selected).map(|d| d.tag.as_str());
            self.derived = project
                .derived
                .iter()
                .map(|d| p.compile_expr(&d.expr, tag).map(|c| c.eval(&values)))
                .collect();
            self.values = p
                .names()
                .iter()
                .cloned()
                .zip(values.iter().copied())
                .collect();
        }

        for (i, d) in project.datasets.iter().enumerate() {
            let shown = i == selected || (view.overlay && d.include);
            if !shown {
                continue;
            }
            let data = &d.data;
            let (x, y, sigma) = (data.x(), data.y(), data.sigma());
            let mut c = Curves {
                name: format!("{} {}", d.tag, data.name),
                color: dataset_color(i),
                inside: Vec::new(),
                outside: Vec::new(),
                errors: Vec::new(),
                model: Vec::new(),
                components: Vec::new(),
                residuals: Vec::new(),
                weighted: false,
                range: data.range,
            };
            for k in 0..x.len().min(y.len()) {
                let Some(p) = pt(x[k], y[k], view) else {
                    continue;
                };
                if data.in_range(x[k]) {
                    c.inside.push(p);
                } else {
                    c.outside.push(p);
                }
                if view.error_bars
                    && let Some(s) = sigma
                        .and_then(|s| s.get(k))
                        .filter(|s| s.is_finite() && **s > 0.0)
                    && let (Some(lo), Some(hi)) =
                        (tf(y[k] - s, view.log_y), tf(y[k] + s, view.log_y))
                {
                    c.errors.push((p[0], lo, hi));
                }
            }

            let job = problem.as_ref().and_then(|p| {
                p.datasets()
                    .iter()
                    .position(|j| j.tag == d.tag)
                    .map(|k| (p, k))
            });
            if let Some((p, k)) = job {
                let model = &p.datasets()[k].model;
                let pv = p.dataset_values(k, &values);
                if let Some((lo, hi)) = data.x_extent() {
                    let n = 800;
                    let grid: Vec<f64> = if view.log_x && lo > 0.0 {
                        let (a, b) = (lo.log10(), hi.log10());
                        (0..n)
                            .map(|i| 10f64.powf(a + (b - a) * i as f64 / (n - 1) as f64))
                            .collect()
                    } else {
                        (0..n)
                            .map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64)
                            .collect()
                    };
                    let mut f = vec![0.0; n];
                    match model.eval(&grid, pv, &mut f) {
                        Ok(()) => {
                            c.model = grid
                                .iter()
                                .zip(&f)
                                .filter_map(|(&x, &y)| pt(x, y, view))
                                .collect();
                            if view.show_components
                                && model.component_names().len() > 1
                                && let Ok(comps) = model.eval_components(&grid, pv)
                            {
                                for (name, ys) in model.component_names().iter().zip(comps) {
                                    let line = grid
                                        .iter()
                                        .zip(&ys)
                                        .filter_map(|(&x, &y)| pt(x, y, view))
                                        .collect();
                                    c.components.push((name.clone(), line));
                                }
                            }
                        }
                        Err(e) => self.error = Some(format!("{}: {e}", d.tag)),
                    }
                }
                let a = &p.datasets()[k].arrays;
                let mut f = vec![0.0; a.x.len()];
                if model.eval(&a.x, pv, &mut f).is_ok() {
                    // Weighted, i.e. what the fit minimises: (y - f)/σ is comparable
                    // across decades of intensity, unlike y - f.
                    c.residuals = (0..a.x.len())
                        .filter_map(|k| Some([tf(a.x[k], view.log_x)?, (a.y[k] - f[k]) * a.w[k]]))
                        .collect();
                    c.weighted = a.w.iter().any(|w| *w != 1.0);
                }
            }
            self.curves.push(c);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, view: &ViewOptions) {
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let total = ui.available_height();
        let residuals = view.show_residuals && self.curves.iter().any(|c| !c.residuals.is_empty());
        let main_h = if residuals {
            (total * 0.72).max(120.0)
        } else {
            total
        };
        let log_label = |mark: GridMark, _: &RangeInclusive<f64>| {
            let v = 10f64.powf(mark.value);
            if (mark.value - mark.value.round()).abs() < 1e-9 {
                format!("1e{}", mark.value.round() as i64)
            } else {
                super::widgets::fmt_num(v)
            }
        };

        // egui_plot's default wants 60 px between x labels, which leaves a
        // narrow plot with a single label; numbers here are short.
        let x_axis = || {
            let hints = AxisHints::new_x().label_spacing(14.0..=40.0);
            if view.log_x {
                hints.formatter(log_label)
            } else {
                hints
            }
        };
        let mut plot = Plot::new("main_plot")
            .custom_x_axes(vec![x_axis()])
            .legend(Legend::default())
            .height(main_h)
            .link_axis("fit_plots", [true, false])
            .link_cursor("fit_plots", [true, false])
            .y_axis_min_width(48.0);
        if view.log_x {
            plot = plot.x_grid_spacer(log_grid_spacer(10));
        }
        if view.log_y {
            plot = plot
                .y_grid_spacer(log_grid_spacer(10))
                .y_axis_formatter(log_label);
        }
        let curves = &self.curves;
        let bounds = plot
            .show(ui, |pui| {
                for c in curves {
                    if let Some((lo, hi)) = c.range
                        && let (Some(lo), Some(hi)) =
                            (tf(lo.min(hi), view.log_x), tf(lo.max(hi), view.log_x))
                    {
                        pui.span(
                            Span::new(t("fit range", "フィット範囲"), lo..=hi)
                                .fill(c.color.gamma_multiply(0.06))
                                .border_width(0.0),
                        );
                    }
                    for (x, lo, hi) in &c.errors {
                        pui.line(
                            Line::new("", vec![[*x, *lo], [*x, *hi]])
                                .color(c.color.gamma_multiply(0.5))
                                .width(1.0)
                                .allow_hover(false),
                        );
                    }
                    pui.points(
                        Points::new(c.name.clone(), c.inside.clone())
                            .color(c.color)
                            .radius(2.0),
                    );
                    if !c.outside.is_empty() {
                        pui.points(
                            Points::new(
                                t(
                                    format!("{} (not fitted)", c.name),
                                    format!("{} (フィット対象外)", c.name),
                                ),
                                c.outside.clone(),
                            )
                            .color(c.color.gamma_multiply(0.3))
                            .radius(2.0),
                        );
                    }
                    for (name, line) in &c.components {
                        pui.line(
                            Line::new(
                                format!("{} {name}", c.name.split(' ').next().unwrap_or("")),
                                line.clone(),
                            )
                            .style(LineStyle::dashed_loose())
                            .width(1.2),
                        );
                    }
                    if !c.model.is_empty() {
                        pui.line(
                            Line::new(
                                format!(
                                    "{} {}",
                                    c.name.split(' ').next().unwrap_or(""),
                                    t("model", "モデル")
                                ),
                                c.model.clone(),
                            )
                            .color(c.color)
                            .width(2.0),
                        );
                    }
                }
                pui.plot_bounds()
            })
            .inner;
        let (lo, hi) = (bounds.min()[0], bounds.max()[0]);
        self.view_x = Some(if view.log_x {
            (10f64.powf(lo), 10f64.powf(hi))
        } else {
            (lo, hi)
        });

        if residuals {
            let label = if curves.iter().any(|c| c.weighted) {
                t("(y − model) / σ", "(y − モデル) / σ")
            } else {
                t("y − model", "y − モデル")
            };
            let mut rplot = Plot::new("residual_plot")
                .custom_x_axes(vec![x_axis()])
                .height(ui.available_height())
                .link_axis("fit_plots", [true, false])
                .link_cursor("fit_plots", [true, false])
                .y_axis_min_width(48.0)
                .y_axis_label(label);
            if view.log_x {
                rplot = rplot.x_grid_spacer(log_grid_spacer(10));
            }
            rplot.show(ui, |pui| {
                pui.hline(HLine::new("", 0.0).color(Color32::GRAY).width(1.0));
                for c in curves {
                    pui.points(
                        Points::new(
                            format!("{} {}", c.name, t("residual", "残差")),
                            c.residuals.clone(),
                        )
                        .color(c.color)
                        .radius(1.5),
                    );
                }
            });
        }
    }
}
