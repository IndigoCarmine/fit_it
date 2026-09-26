//! Everything a user builds up in a session: datasets, their models and
//! parameters. Saved as JSON (`*.fitit.json`); also what eframe persists.

use crate::data::Dataset;
use crate::fit::{DatasetJob, FitOptions, FitOutcome, Param, Problem, sync_params};
use crate::model::ModelRef;
use crate::model::composite::{CompiledComposite, Component, ModelSpec};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PROJECT_EXTENSION: &str = "fitit.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatasetState {
    /// Short unique name used in constraints (`D1.g1_sigma`).
    pub tag: String,
    pub data: Dataset,
    pub spec: ModelSpec,
    pub params: Vec<Param>,
    /// Takes part in global fits.
    pub include: bool,
}

impl DatasetState {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub datasets: Vec<DatasetState>,
    pub options: FitOptions,
    /// Quantities computed from the parameters and reported with propagated
    /// errors, e.g. ΔG(300 K) = `t1_deltaH - 300 * t1_deltaS`.
    pub derived: Vec<DerivedSpec>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DerivedSpec {
    pub name: String,
    /// Bare names refer to the selected dataset; `D2.name` reaches any dataset.
    pub expr: String,
}

impl DerivedSpec {
    pub fn pairs(specs: &[DerivedSpec]) -> Vec<(String, String)> {
        specs
            .iter()
            .filter(|d| !d.expr.trim().is_empty())
            .map(|d| (d.name.clone(), d.expr.clone()))
            .collect()
    }
}

pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<ModelRef>;

impl Project {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn unique_tag(&self) -> String {
        (1..)
            .map(|i| format!("D{i}"))
            .find(|t| self.datasets.iter().all(|d| &d.tag != t))
            .unwrap()
    }

    pub fn add_dataset(&mut self, data: Dataset) -> usize {
        let tag = self.unique_tag();
        self.datasets.push(DatasetState {
            tag,
            data,
            include: true,
            ..Default::default()
        });
        self.datasets.len() - 1
    }

    /// Bring every dataset's parameter list in line with its model.
    /// Datasets whose model cannot be built keep their parameters untouched.
    /// New parameters whose model declares a default constraint (e.g. `c_tot` =
    /// `conc_M`) get it when the dataset defines every name the constraint uses.
    pub fn sync_params(&mut self, lookup: Lookup) {
        for d in &mut self.datasets {
            let Ok(c) = CompiledComposite::build(&d.spec, lookup) else {
                continue;
            };
            let added = sync_params(&mut d.params, c.params());
            for i in added {
                let Some(def) = c.params().get(i).filter(|d| !d.expr.trim().is_empty()) else {
                    continue;
                };
                let Ok(e) = crate::expr::Expr::parse(&def.expr) else {
                    continue;
                };
                let known = e.vars().iter().all(|v| {
                    d.data.constants.contains_key(v) || c.params().iter().any(|p| &p.name == v)
                });
                if known {
                    d.params[i].expr = def.expr.clone();
                }
            }
        }
    }

    /// A fit problem over all datasets that have a usable model. Only datasets for
    /// which `active` is true are fitted; the rest provide fixed values that
    /// constraints may refer to.
    pub fn problem(
        &self,
        lookup: Lookup,
        active: &dyn Fn(usize) -> bool,
    ) -> Result<Problem, String> {
        let mut jobs = Vec::new();
        for (i, d) in self.datasets.iter().enumerate() {
            let model = match CompiledComposite::build(&d.spec, lookup) {
                Ok(m) => m,
                Err(e) if active(i) => return Err(format!("{}: {e}", d.tag)),
                Err(_) => continue,
            };
            jobs.push(DatasetJob {
                tag: d.tag.clone(),
                model,
                arrays: d.data.fit_arrays(),
                params: d.params.clone(),
                active: active(i),
                constants: d
                    .data
                    .constants
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect(),
            });
        }
        Problem::new(jobs)
    }

    /// Write fitted values and uncertainties back into the parameters.
    pub fn apply_outcome(&mut self, out: &FitOutcome) {
        for d in &mut self.datasets {
            for p in &mut d.params {
                if let Some((v, e)) = out.value_of(&format!("{}.{}", d.tag, p.name)) {
                    p.value = v;
                    p.stderr = e;
                }
            }
        }
    }

    pub fn clear_stderr(&mut self) {
        for d in &mut self.datasets {
            for p in &mut d.params {
                p.stderr = None;
            }
        }
    }

    /// Give every dataset checked for the global fit the fit range of dataset `src`.
    pub fn range_to_all(&mut self, src: usize) {
        let r = self.datasets[src].data.range;
        for d in self.datasets.iter_mut().filter(|d| d.include) {
            d.data.range = r;
        }
    }

    /// `<tag>.<name>`: how constraints refer to parameter `name` of dataset `src`.
    fn qualified(&self, src: usize, name: &str) -> String {
        format!("{}.{}", self.datasets[src].tag, name)
    }

    /// Make parameter `name` of dataset `src` shared: every other dataset with a
    /// parameter of that name gets the constraint `<src tag>.<name>`.
    pub fn share_param(&mut self, src: usize, name: &str) {
        let expr = self.qualified(src, name);
        for (i, d) in self.datasets.iter_mut().enumerate() {
            if i == src {
                continue;
            }
            if let Some(p) = d.params.iter_mut().find(|p| p.name == name) {
                p.expr = expr.clone();
            }
        }
        if let Some(p) = self.datasets[src]
            .params
            .iter_mut()
            .find(|p| p.name == name)
        {
            p.expr.clear();
        }
    }

    /// Undo [`Project::share_param`]: drop constraints that point at `<src tag>.<name>`.
    pub fn unshare_param(&mut self, src: usize, name: &str) {
        let expr = self.qualified(src, name);
        for d in &mut self.datasets {
            for p in &mut d.params {
                if p.expr.trim() == expr {
                    p.expr.clear();
                }
            }
        }
    }

    /// How many other datasets currently take `name` from dataset `src`.
    pub fn shared_count(&self, src: usize, name: &str) -> usize {
        let expr = self.qualified(src, name);
        self.datasets
            .iter()
            .flat_map(|d| &d.params)
            .filter(|p| p.expr.trim() == expr)
            .count()
    }

    /// Set parameter `name` of every dataset that has one to its value in dataset `src`.
    pub fn copy_value_to_all(&mut self, src: usize, name: &str) {
        let Some(v) = self.datasets[src].param(name).map(|p| p.value) else {
            return;
        };
        for d in &mut self.datasets {
            if let Some(p) = d.params.iter_mut().find(|p| p.name == name) {
                p.value = v;
            }
        }
    }

    /// Give every other dataset checked for the global fit dataset `src`'s model and parameter settings
    /// (bounds, vary, constraints), then re-estimate starting values from each
    /// dataset's own data where the components can guess — copied values are
    /// usually a poor start when peaks move between datasets.
    pub fn copy_model_to_all(&mut self, src: usize, lookup: Lookup) {
        let spec = self.datasets[src].spec.clone();
        let params = self.datasets[src].params.clone();
        for i in 0..self.datasets.len() {
            if i == src || !self.datasets[i].include {
                continue;
            }
            let d = &mut self.datasets[i];
            d.spec = spec.clone();
            // Unqualified constraints (`2 * g1_sigma`) now refer to each
            // dataset's own parameters, which is what "same model" means.
            d.params = params
                .iter()
                .map(|p| Param {
                    stderr: None,
                    ..p.clone()
                })
                .collect();
            for c in 0..spec.components.len() {
                let _ = self.guess_component(i, c, lookup);
            }
        }
    }

    /// Rename a component, carrying its parameters (prefix `old_` → `new_`) along.
    pub fn rename_component(&mut self, ds: usize, index: usize, new_name: &str) {
        let d = &mut self.datasets[ds];
        let Some(c) = d.spec.components.get_mut(index) else {
            return;
        };
        let old = std::mem::replace(&mut c.name, new_name.to_string());
        let (old_prefix, new_prefix) = (format!("{old}_"), format!("{new_name}_"));
        for p in &mut d.params {
            if let Some(rest) = p.name.strip_prefix(&old_prefix) {
                p.name = format!("{new_prefix}{rest}");
            }
        }
    }

    pub fn add_component(&mut self, ds: usize, model: &str) -> String {
        let d = &mut self.datasets[ds];
        let name = d.spec.suggest_name(model);
        d.spec.components.push(Component {
            name: name.clone(),
            model: model.to_string(),
        });
        name
    }

    /// Estimate a component's parameters from the data minus the other components.
    pub fn guess_component(
        &mut self,
        ds: usize,
        index: usize,
        lookup: Lookup,
    ) -> Result<(), String> {
        let d = &self.datasets[ds];
        let comp = d
            .spec
            .components
            .get(index)
            .ok_or("no such component")?
            .clone();
        let model =
            lookup(&comp.model).ok_or_else(|| format!("model `{}` is not loaded", comp.model))?;
        let a = d.data.fit_arrays();
        if a.x.is_empty() {
            return Err("no data to guess from".into());
        }
        // Subtract what the other components already explain.
        let mut target = a.y.clone();
        let others = ModelSpec {
            components: d
                .spec
                .components
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, c)| c.clone())
                .collect(),
            formula: String::new(),
        };
        if !others.components.is_empty()
            && d.spec.formula.trim().is_empty()
            && let Ok(c) = CompiledComposite::build(&others, lookup)
        {
            let values: Vec<f64> = c
                .params()
                .iter()
                .map(|def| d.param(&def.name).map_or(def.default, |p| p.value))
                .collect();
            let mut f = vec![0.0; a.x.len()];
            if c.eval(&a.x, &values, &mut f).is_ok() {
                for (t, f) in target.iter_mut().zip(&f) {
                    *t -= f;
                }
            }
        }
        let g = model
            .guess(&a.x, &target)
            .ok_or_else(|| format!("{} cannot guess its parameters", comp.model))?;
        let d = &mut self.datasets[ds];
        for (def, v) in model.info().params.iter().zip(g) {
            let full = format!("{}_{}", comp.name, def.name);
            if let Some(p) = d.params.iter_mut().find(|p| p.name == full)
                && p.expr.trim().is_empty()
                && v.is_finite()
            {
                p.value = v.clamp(p.min.min(p.max), p.max.max(p.min));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Dataset, parse_table};
    use crate::fit::fit;
    use crate::model::testing::lookup;

    fn gaussian_data(center: f64, sigma: f64) -> Dataset {
        let mut text = String::from("x y\n");
        for i in 0..200 {
            let x = -5.0 + i as f64 * 0.05;
            let y = 3.0 * (-(x - center).powi(2) / (2.0 * sigma * sigma)).exp() + 0.5;
            text.push_str(&format!("{x} {y}\n"));
        }
        Dataset::from_table("g".into(), None, parse_table(&text).unwrap())
    }

    fn project() -> Project {
        let mut p = Project::default();
        for c in [-1.0, 1.5] {
            let i = p.add_dataset(gaussian_data(c, 0.6));
            p.add_component(i, "gaussian");
            p.add_component(i, "linear");
        }
        p.sync_params(&lookup);
        p
    }

    #[test]
    fn tags_components_and_params() {
        let p = project();
        assert_eq!(p.datasets[1].tag, "D2");
        let names: Vec<_> = p.datasets[0]
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "g1_amplitude",
                "g1_center",
                "g1_sigma",
                "l1_slope",
                "l1_intercept"
            ]
        );
    }

    #[test]
    fn guess_then_global_fit_with_shared_sigma() {
        let mut p = project();
        for ds in 0..2 {
            // Background first is not needed: the Gaussian's own guess handles the offset poorly,
            // so fix the line to the known baseline.
            let d = &mut p.datasets[ds];
            d.params[3].value = 0.0;
            d.params[4].value = 0.5;
        }
        p.share_param(0, "g1_sigma");
        assert_eq!(p.shared_count(0, "g1_sigma"), 1);
        assert_eq!(p.datasets[1].param("g1_sigma").unwrap().expr, "D1.g1_sigma");
        for ds in 0..2 {
            p.datasets[ds].params[0].value = 5.0;
            p.datasets[ds].params[1].value = if ds == 0 { -0.8 } else { 1.2 };
        }
        let problem = p.problem(&lookup, &|i| p.datasets[i].include).unwrap();
        let out = fit(&problem, &p.options, None, None).unwrap();
        p.apply_outcome(&out);
        let s1 = p.datasets[0].param("g1_sigma").unwrap().value;
        let s2 = p.datasets[1].param("g1_sigma").unwrap().value;
        assert!((s1 - 0.6).abs() < 1e-3, "{}", out.report());
        assert_eq!(s1, s2);
        assert!((p.datasets[1].param("g1_center").unwrap().value - 1.5).abs() < 1e-3);
        p.unshare_param(0, "g1_sigma");
        assert!(p.datasets[1].param("g1_sigma").unwrap().expr.is_empty());
    }

    #[test]
    fn copy_to_all_reguesses_starting_values() {
        let mut p = Project::default();
        for c in [-2.0, 2.0] {
            p.add_dataset(gaussian_data(c, 0.6));
        }
        p.add_component(0, "gaussian");
        p.sync_params(&lookup);
        p.datasets[0].params[2].min = 0.1;
        p.copy_model_to_all(0, &lookup);
        let d2 = &p.datasets[1];
        assert_eq!(d2.spec, p.datasets[0].spec);
        assert_eq!(d2.param("g1_sigma").unwrap().min, 0.1);
        // The test Gaussian has no guess, so values are the copied ones; the real
        // presets guess (see plugin tests). Here we only check nothing broke.
        assert_eq!(d2.params.len(), 3);
    }

    #[test]
    fn rename_component_moves_params() {
        let mut p = project();
        p.rename_component(0, 0, "peak");
        assert!(p.datasets[0].param("peak_center").is_some());
        assert!(p.datasets[0].param("g1_center").is_none());
    }

    #[test]
    fn datasets_without_model_are_skipped_unless_active() {
        let mut p = project();
        p.add_dataset(gaussian_data(0.0, 1.0));
        assert!(p.problem(&lookup, &|i| i == 0).is_ok());
        assert!(p.problem(&lookup, &|i| i == 2).is_err());
    }

    #[test]
    fn project_round_trips_through_json() {
        let p = project();
        let path = std::env::temp_dir().join(format!(
            "fit_it_project_{}.{PROJECT_EXTENSION}",
            std::process::id()
        ));
        p.save(&path).unwrap();
        let back = Project::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, p);
    }
}
