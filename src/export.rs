//! Exporting fit results: per-dataset curves, and all datasets side by side as CSV.

use crate::project::{Lookup, Project};

/// Dataset colours (Okabe–Ito; readable in light and dark themes and in print),
/// shared by the app and the PDF report.
pub const PALETTE: [(u8, u8, u8); 7] = [
    (0, 114, 178),
    (213, 94, 0),
    (0, 158, 115),
    (204, 121, 167),
    (230, 159, 0),
    (86, 180, 233),
    (120, 120, 120),
];

/// Everything needed to plot or tabulate one dataset's fit.
#[derive(Clone, Debug, Default)]
pub struct DatasetCurves {
    pub tag: String,
    pub name: String,
    pub x_label: String,
    pub y_label: String,
    /// Measured points (finite x, y), in file order.
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub sigma: Option<Vec<f64>>,
    /// Point lies inside the fit range.
    pub fitted: Vec<bool>,
    /// Model and its components at the measured x (empty without a model).
    pub model: Vec<f64>,
    pub components: Vec<(String, Vec<f64>)>,
    /// Smooth model curve on a dense grid across the data (for plotting).
    pub grid_x: Vec<f64>,
    pub grid_model: Vec<f64>,
    pub grid_components: Vec<(String, Vec<f64>)>,
    /// Weighted residuals (y - f) / σ at the points in the fit range.
    pub residual_x: Vec<f64>,
    pub residual: Vec<f64>,
    pub residual_weighted: bool,
}

/// Datasets to export: those checked for the global fit, else just `selected`.
pub fn export_set(project: &Project, selected: usize) -> Vec<usize> {
    let checked: Vec<usize> = (0..project.datasets.len())
        .filter(|&i| project.datasets[i].include)
        .collect();
    if checked.is_empty() && selected < project.datasets.len() {
        vec![selected]
    } else {
        checked
    }
}

/// Evaluate the current model of each dataset in `which` (constraints applied).
/// With `log_grid`, the dense curve is spaced logarithmically when x > 0.
pub fn dataset_curves(
    project: &Project,
    lookup: Lookup,
    which: &[usize],
    grid_n: usize,
    log_grid: bool,
) -> Result<Vec<DatasetCurves>, String> {
    let problem = project.problem(lookup, &|_| false).ok();
    let values = problem
        .as_ref()
        .map(|p| p.current_values())
        .unwrap_or_default();
    let mut out = Vec::new();
    for &i in which {
        let d = &project.datasets[i];
        let data = &d.data;
        let header = |c: usize| data.table.headers.get(c).cloned().unwrap_or_default();
        let mut c = DatasetCurves {
            tag: d.tag.clone(),
            name: data.name.clone(),
            x_label: header(data.x_col),
            y_label: header(data.y_col),
            ..Default::default()
        };
        let (xs, ys, sig) = (data.x(), data.y(), data.sigma());
        let mut sigma = Vec::new();
        for k in 0..xs.len().min(ys.len()) {
            if xs[k].is_finite() && ys[k].is_finite() {
                c.x.push(xs[k]);
                c.y.push(ys[k]);
                c.fitted.push(data.in_range(xs[k]));
                sigma.push(sig.and_then(|s| s.get(k).copied()).unwrap_or(f64::NAN));
            }
        }
        if sig.is_some() {
            c.sigma = Some(sigma);
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
            let mut f = vec![0.0; c.x.len()];
            model
                .eval(&c.x, pv, &mut f)
                .map_err(|e| format!("{}: {e}", d.tag))?;
            c.model = f;
            let names = model.component_names();
            if names.len() > 1 {
                let comps = model.eval_components(&c.x, pv)?;
                c.components = names.iter().cloned().zip(comps).collect();
            }
            if let Some((lo, hi)) = data.x_extent()
                && grid_n > 1
            {
                c.grid_x = if log_grid && lo > 0.0 {
                    let (a, b) = (lo.log10(), hi.log10());
                    (0..grid_n)
                        .map(|i| 10f64.powf(a + (b - a) * i as f64 / (grid_n - 1) as f64))
                        .collect()
                } else {
                    (0..grid_n)
                        .map(|i| lo + (hi - lo) * i as f64 / (grid_n - 1) as f64)
                        .collect()
                };
                let mut g = vec![0.0; grid_n];
                model.eval(&c.grid_x, pv, &mut g)?;
                c.grid_model = g;
                if names.len() > 1 {
                    let comps = model.eval_components(&c.grid_x, pv)?;
                    c.grid_components = names.iter().cloned().zip(comps).collect();
                }
            }
            let a = &p.datasets()[k].arrays;
            let mut fa = vec![0.0; a.x.len()];
            if model.eval(&a.x, pv, &mut fa).is_ok() {
                c.residual_x = a.x.clone();
                c.residual = (0..a.x.len()).map(|j| (a.y[j] - fa[j]) * a.w[j]).collect();
                c.residual_weighted = a.w.iter().any(|w| *w != 1.0);
            }
        }
        out.push(c);
    }
    Ok(out)
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// All datasets side by side: for each, `x, y, [sigma], model, residual,
/// in_fit_range, components...` columns, prefixed with the dataset tag. Shorter
/// datasets leave their cells empty. A first header row names the datasets.
pub fn side_by_side_csv(curves: &[DatasetCurves]) -> String {
    let mut names = Vec::new();
    let mut header = Vec::new();
    for c in curves {
        let mut cols = vec!["x", "y"];
        if c.sigma.is_some() {
            cols.push("sigma");
        }
        if !c.model.is_empty() {
            cols.extend(["model", "residual"]);
        }
        cols.push("in_fit_range");
        let comp_names: Vec<&str> = c.components.iter().map(|(n, _)| n.as_str()).collect();
        let width = cols.len() + comp_names.len();
        names.push(csv_field(&format!("{} {}", c.tag, c.name)));
        names.extend(std::iter::repeat_n(String::new(), width - 1));
        for col in cols.into_iter().chain(comp_names) {
            header.push(csv_field(&format!("{}_{col}", c.tag)));
        }
    }
    let rows = curves.iter().map(|c| c.x.len()).max().unwrap_or(0);
    let mut s = String::new();
    s.push_str(&names.join(","));
    s.push('\n');
    s.push_str(&header.join(","));
    s.push('\n');
    for r in 0..rows {
        let mut cells: Vec<String> = Vec::new();
        for c in curves {
            let n = 2
                + usize::from(c.sigma.is_some())
                + if c.model.is_empty() { 0 } else { 2 }
                + 1
                + c.components.len();
            if r >= c.x.len() {
                cells.extend(std::iter::repeat_n(String::new(), n));
                continue;
            }
            cells.push(c.x[r].to_string());
            cells.push(c.y[r].to_string());
            if let Some(s) = &c.sigma {
                cells.push(if s[r].is_finite() {
                    s[r].to_string()
                } else {
                    String::new()
                });
            }
            if !c.model.is_empty() {
                cells.push(c.model[r].to_string());
                cells.push((c.y[r] - c.model[r]).to_string());
            }
            cells.push(u8::from(c.fitted[r]).to_string());
            for (_, v) in &c.components {
                cells.push(v[r].to_string());
            }
        }
        s.push_str(&cells.join(","));
        s.push('\n');
    }
    s
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::data::{Dataset, parse_table};
    use crate::model::testing::lookup;

    pub(crate) fn two_dataset_project() -> Project {
        let mut p = Project::default();
        for (n, c) in [(30, 0.0), (20, 1.0)] {
            let mut text = String::from("x y\n");
            for i in 0..n {
                let x = -3.0 + 6.0 * i as f64 / (n - 1) as f64;
                text.push_str(&format!("{x} {}\n", (-(x - c) * (x - c)).exp() + 0.1 * x));
            }
            let i = p.add_dataset(Dataset::from_table(
                format!("set{c}"),
                None,
                parse_table(&text).unwrap(),
            ));
            p.add_component(i, "gaussian");
            p.add_component(i, "linear");
        }
        p.sync_params(&lookup);
        p
    }

    #[test]
    fn curves_and_side_by_side_csv() {
        let p = two_dataset_project();
        let set = export_set(&p, 0);
        assert_eq!(set, [0, 1]);
        let curves = dataset_curves(&p, &lookup, &set, 50, false).unwrap();
        assert_eq!(curves[0].x.len(), 30);
        assert_eq!(curves[1].grid_x.len(), 50);
        assert_eq!(curves[0].components.len(), 2);
        let csv = side_by_side_csv(&curves);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2 + 30);
        assert_eq!(
            lines[1],
            "D1_x,D1_y,D1_model,D1_residual,D1_in_fit_range,D1_g1,D1_l1,D2_x,D2_y,D2_model,D2_residual,D2_in_fit_range,D2_g1,D2_l1"
        );
        assert!(lines[0].starts_with("D1 set0,,,,,,,D2 set1"));
        // Rows past the shorter dataset have empty D2 cells.
        assert!(lines[25].ends_with(",,,,,,"));
        assert_eq!(lines[25].split(',').count(), 14);
    }

    #[test]
    fn export_set_falls_back_to_selected() {
        let mut p = two_dataset_project();
        for d in &mut p.datasets {
            d.include = false;
        }
        assert_eq!(export_set(&p, 1), [1]);
    }
}
