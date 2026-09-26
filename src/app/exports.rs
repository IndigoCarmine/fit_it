//! File > Export: curves as CSV, the text report and the PDF report.

use super::FitApp;
use super::clock::utc_datetime;
use super::files::save_dialog;
use super::i18n::t;
use crate::export;
use crate::plugin;
use crate::project::{Lookup, Project};

/// Data and model of dataset `ds` as CSV: `x, y, [sigma], model, residual,
/// in_fit_range, components...`.
fn dataset_csv(project: &Project, lookup: Lookup, ds: usize) -> Result<String, String> {
    let d = &project.datasets[ds];
    let problem = project.problem(lookup, &|_| false)?;
    let k = problem
        .datasets()
        .iter()
        .position(|j| j.tag == d.tag)
        .ok_or(t(
            "this dataset has no model",
            "このデータセットにはモデルがありません",
        ))?;
    let model = &problem.datasets()[k].model;
    let values = problem.current_values();
    let pv = problem.dataset_values(k, &values);
    let (x, y) = (d.data.x(), d.data.y());
    let n = x.len().min(y.len());
    let mut f = vec![0.0; n];
    model.eval(&x[..n], pv, &mut f)?;
    let comps = model.eval_components(&x[..n], pv)?;
    let sigma = d.data.sigma();
    let mut out = String::from("x,y");
    if sigma.is_some() {
        out.push_str(",sigma");
    }
    out.push_str(",model,residual,in_fit_range");
    for c in model.component_names() {
        out.push_str(&format!(",{c}"));
    }
    out.push('\n');
    for i in 0..n {
        out.push_str(&format!("{},{}", x[i], y[i]));
        if let Some(s) = sigma {
            out.push_str(&format!(",{}", s.get(i).copied().unwrap_or(f64::NAN)));
        }
        out.push_str(&format!(
            ",{},{},{}",
            f[i],
            y[i] - f[i],
            u8::from(d.data.in_range(x[i]))
        ));
        for c in &comps {
            out.push_str(&format!(",{}", c[i]));
        }
        out.push('\n');
    }
    Ok(out)
}

impl FitApp {
    pub(super) fn export_curves(&mut self) {
        let Some(ds) = self.current() else { return };
        let file_name = format!("{}_fit.csv", self.project.datasets[ds].tag);
        let Some(path) = save_dialog("CSV", &["csv"], &file_name) else {
            return;
        };
        let lookup = self.lookup();
        let result = dataset_csv(&self.project, &lookup, ds)
            .and_then(|csv| std::fs::write(&path, csv).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.set_status(t(
                format!("Exported {}", path.display()),
                format!("{} に書き出しました", path.display()),
            )),
            Err(e) => self.set_error(e),
        }
    }

    /// Tags of the datasets an export covers (checked for the global fit, else the selected one).
    fn export_tags(&self, set: &[usize]) -> String {
        set.iter()
            .map(|&i| self.project.datasets[i].tag.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Every dataset checked for the global fit, side by side in one CSV.
    pub(super) fn export_global_csv(&mut self) {
        let set = export::export_set(&self.project, self.selected);
        if set.is_empty() {
            return;
        }
        let Some(path) = save_dialog("CSV", &["csv"], "global_fit.csv") else {
            return;
        };
        let lookup = self.lookup();
        let result = export::dataset_curves(&self.project, &lookup, &set, 0, false)
            .map(|c| export::side_by_side_csv(&c))
            .and_then(|csv| std::fs::write(&path, csv).map_err(|e| e.to_string()));
        let tags = self.export_tags(&set);
        match result {
            Ok(()) => self.set_status(t(
                format!("Exported {tags} side by side to {}", path.display()),
                format!("{tags} を並べて {} に書き出しました", path.display()),
            )),
            Err(e) => self.set_error(e),
        }
    }

    /// Lines under the PDF report's title: fit, datasets, project, creation time.
    fn report_info(&self, set: &[usize]) -> Vec<String> {
        let mut info = Vec::new();
        if let Some(first) = self.rt.report.lines().next() {
            info.push(first.to_string());
        } else {
            info.push("Not fitted yet: curves use the current parameter values".into());
        }
        info.push(format!("Datasets: {}", self.export_tags(set)));
        if let Some(p) = &self.project_path {
            info.push(format!("Project: {}", p.display()));
        }
        info.push(format!(
            "Created: {} (fit_it {})",
            utc_datetime(),
            env!("CARGO_PKG_VERSION")
        ));
        info
    }

    /// PDF report: overview plot, one page per dataset (fit, residuals,
    /// parameters) and the full fit report.
    pub(super) fn export_pdf(&mut self) {
        let set = export::export_set(&self.project, self.selected);
        if set.is_empty() {
            return;
        }
        let Some(path) = save_dialog("PDF", &["pdf"], "fit_report.pdf") else {
            return;
        };
        let lookup = self.lookup();
        let curves =
            match export::dataset_curves(&self.project, &lookup, &set, 600, self.view.log_x) {
                Ok(c) => c,
                Err(e) => {
                    self.set_error(e);
                    return;
                }
            };
        let input = crate::report_pdf::ReportInput {
            title: "fit_it report".into(),
            info: self.report_info(&set),
            curves: &curves,
            colors: set.clone(),
            params: set
                .iter()
                .map(|&i| self.project.datasets[i].params.clone())
                .collect(),
            report_text: &self.rt.report,
            log_x: self.view.log_x,
            log_y: self.view.log_y,
        };
        let bytes = crate::report_pdf::render(&input);
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.status_saved(&path);
                plugin::open_in_os(&path);
            }
            Err(e) => self.set_error(e.to_string()),
        }
    }

    pub(super) fn export_report(&mut self) {
        if self.rt.report.is_empty() {
            return;
        }
        if let Some(path) = save_dialog(t("Text", "テキスト"), &["txt"], "fit_report.txt") {
            match std::fs::write(&path, &self.rt.report) {
                Ok(()) => self.status_saved(&path),
                Err(e) => self.set_error(e.to_string()),
            }
        }
    }
}
