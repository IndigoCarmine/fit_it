//! Combining models: named components joined by a formula.
//!
//! Each component gets a short name (`g1`, `bg`, ...). Its parameters are exposed
//! as `<name>_<param>` (`g1_center`), like lmfit prefixes. The formula combines
//! component outputs point-wise: `(g1 + g2) * decay + bg`. An empty formula
//! means the sum of all components, which is what you want most of the time.

use super::{ModelRef, ParamDef, is_identifier};
use crate::expr::{Compiled, Expr};
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    /// Short name used in the formula and as parameter prefix.
    pub name: String,
    /// Registry name of the model.
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSpec {
    pub components: Vec<Component>,
    pub formula: String,
}

impl ModelSpec {
    /// The formula actually used: the user's, or the implicit sum.
    pub fn effective_formula(&self) -> String {
        if self.formula.trim().is_empty() {
            self.components
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(" + ")
        } else {
            self.formula.clone()
        }
    }

    /// A fresh component name for `model`: its first letter plus a counter (`g1`, `g2`, `l1`).
    pub fn suggest_name(&self, model: &str) -> String {
        let stem: String = model
            .chars()
            .find(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_lowercase().to_string())
            .unwrap_or_else(|| "m".into());
        (1..)
            .map(|i| format!("{stem}{i}"))
            .find(|n| self.components.iter().all(|c| &c.name != n))
            .unwrap()
    }
}

/// A [`ModelSpec`] resolved against the registry and ready to evaluate.
#[derive(Clone)]
pub struct CompiledComposite {
    parts: Vec<(ModelRef, Range<usize>)>,
    names: Vec<String>,
    /// Slots: one per component, then `x`.
    formula: Compiled,
    params: Vec<ParamDef>,
}

impl CompiledComposite {
    pub fn build(
        spec: &ModelSpec,
        lookup: &dyn Fn(&str) -> Option<ModelRef>,
    ) -> Result<Self, String> {
        if spec.components.is_empty() {
            return Err("the model has no components".into());
        }
        let mut parts = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut params = Vec::new();
        for c in &spec.components {
            if !is_identifier(&c.name) {
                return Err(format!("`{}` is not a valid component name", c.name));
            }
            if c.name == "x" || names.contains(&c.name) {
                return Err(format!("component name `{}` is used twice", c.name));
            }
            let model =
                lookup(&c.model).ok_or_else(|| format!("model `{}` is not loaded", c.model))?;
            let start = params.len();
            for p in &model.info().params {
                params.push(ParamDef {
                    name: format!("{}_{}", c.name, p.name),
                    ..p.clone()
                });
            }
            parts.push((model, start..params.len()));
            names.push(c.name.clone());
        }
        let expr = Expr::parse(&spec.effective_formula()).map_err(|e| format!("formula: {e}"))?;
        let formula = expr
            .compile(&|n| {
                if n == "x" {
                    Some(names.len())
                } else {
                    names.iter().position(|m| m == n)
                }
            })
            .map_err(|e| format!("formula: {e}"))?;
        Ok(Self {
            parts,
            names,
            formula,
            params,
        })
    }

    pub fn params(&self) -> &[ParamDef] {
        &self.params
    }

    pub fn component_names(&self) -> &[String] {
        &self.names
    }

    /// Output of each component separately.
    pub fn eval_components(&self, x: &[f64], p: &[f64]) -> Result<Vec<Vec<f64>>, String> {
        self.parts
            .iter()
            .zip(&self.names)
            .map(|((m, range), name)| {
                let mut out = vec![0.0; x.len()];
                m.eval(x, &p[range.clone()], &mut out)
                    .map_err(|e| format!("{name} ({}): {e}", m.info().name))?;
                Ok(out)
            })
            .collect()
    }

    pub fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
        let comps = self.eval_components(x, p)?;
        let mut slots = vec![0.0; comps.len() + 1];
        for (i, o) in out.iter_mut().enumerate() {
            for (s, c) in slots.iter_mut().zip(&comps) {
                *s = c[i];
            }
            slots[comps.len()] = x[i];
            *o = self.formula.eval(&slots);
        }
        Ok(())
    }

    /// Ask each component for initial values; `None` entries mean "no guess".
    pub fn guess(&self, x: &[f64], y: &[f64]) -> Vec<Option<f64>> {
        let mut out = vec![None; self.params.len()];
        for (m, range) in &self.parts {
            if let Some(g) = m.guess(x, y)
                && g.len() == range.len()
            {
                for (slot, v) in out[range.clone()].iter_mut().zip(g) {
                    *slot = v.is_finite().then_some(v);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::lookup;

    fn spec(formula: &str) -> ModelSpec {
        ModelSpec {
            components: vec![
                Component {
                    name: "g1".into(),
                    model: "gaussian".into(),
                },
                Component {
                    name: "bg".into(),
                    model: "linear".into(),
                },
            ],
            formula: formula.into(),
        }
    }

    #[test]
    fn prefixes_parameters() {
        let c = CompiledComposite::build(&spec(""), &lookup).unwrap();
        let names: Vec<_> = c.params().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "g1_amplitude",
                "g1_center",
                "g1_sigma",
                "bg_slope",
                "bg_intercept"
            ]
        );
    }

    #[test]
    fn empty_formula_sums_and_custom_formula_combines() {
        let p = [2.0, 0.0, 1.0, 0.5, 1.0];
        let x = [0.0, 1.0];
        let sum = CompiledComposite::build(&spec(""), &lookup).unwrap();
        let prod = CompiledComposite::build(&spec("g1 * bg + x"), &lookup).unwrap();
        let comps = sum.eval_components(&x, &p).unwrap();
        let (mut a, mut b) = ([0.0; 2], [0.0; 2]);
        sum.eval(&x, &p, &mut a).unwrap();
        prod.eval(&x, &p, &mut b).unwrap();
        for i in 0..2 {
            assert!((a[i] - (comps[0][i] + comps[1][i])).abs() < 1e-12);
            assert!((b[i] - (comps[0][i] * comps[1][i] + x[i])).abs() < 1e-12);
        }
    }

    #[test]
    fn reports_bad_specs() {
        assert!(CompiledComposite::build(&spec("g1 + nope"), &lookup).is_err());
        let mut s = spec("");
        s.components[1].name = "g1".into();
        assert!(CompiledComposite::build(&s, &lookup).is_err());
        s.components[1] = Component {
            name: "q".into(),
            model: "missing".into(),
        };
        assert!(CompiledComposite::build(&s, &lookup).is_err());
    }

    #[test]
    fn suggests_unused_names() {
        let s = spec("");
        assert_eq!(s.suggest_name("gaussian"), "g2");
        assert_eq!(s.suggest_name("Lorentzian"), "l1");
    }
}
