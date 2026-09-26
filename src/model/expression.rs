//! Formula models (lmfit's `ExpressionModel`): `y = f(x; params)` written as text.
//!
//! On disk they are `.fexpr` files (TOML) living in the preset or plugin folders:
//!
//! ```toml
//! name = "Stretched exponential"
//! category = "Decay"
//! description = "KWW relaxation"
//! formula = "a * exp(-(x / tau) ^ beta) + c"
//!
//! [[params]]
//! name = "tau"
//! default = 1.0
//! min = 0.0
//! ```
//!
//! Every identifier in the formula other than `x` is a parameter; `[[params]]`
//! entries only refine defaults, bounds and units.

use super::{Model, ModelInfo, ParamDef};
use crate::expr::{Compiled, Expr};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExprParamSpec {
    pub name: String,
    pub default: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: String,
    pub description: String,
    /// `false` = fixed by default.
    pub vary: Option<bool>,
    /// Default constraint, e.g. `conc_M`.
    pub expr: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExprModelSpec {
    pub name: String,
    pub category: String,
    pub description: String,
    pub formula: String,
    pub params: Vec<ExprParamSpec>,
}

impl ExprModelSpec {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Parameter names the formula uses, in order of first appearance.
    pub fn detected_params(&self) -> Result<Vec<String>, String> {
        let e = Expr::parse(&self.formula)?;
        Ok(e.vars().into_iter().filter(|v| v != "x").collect())
    }
}

pub struct ExprModel {
    info: ModelInfo,
    compiled: Compiled,
}

impl ExprModel {
    pub fn new(spec: &ExprModelSpec) -> Result<Self, String> {
        if spec.name.trim().is_empty() {
            return Err("expression model needs a `name`".into());
        }
        let expr = Expr::parse(&spec.formula).map_err(|e| format!("formula: {e}"))?;
        let names: Vec<String> = expr.vars().into_iter().filter(|v| v != "x").collect();
        for n in &names {
            if n.contains('.') {
                return Err(format!("formula: `{n}` is not a valid parameter name"));
            }
        }
        // Slot 0 is x, parameters follow.
        let compiled = expr.compile(&|n| {
            if n == "x" {
                Some(0)
            } else {
                names.iter().position(|m| m == n).map(|i| i + 1)
            }
        })?;
        let params = names
            .iter()
            .map(|n| {
                let s = spec.params.iter().find(|p| &p.name == n);
                ParamDef {
                    name: n.clone(),
                    unit: s.map(|s| s.unit.clone()).unwrap_or_default(),
                    description: s.map(|s| s.description.clone()).unwrap_or_default(),
                    default: s.and_then(|s| s.default).unwrap_or(1.0),
                    min: s.and_then(|s| s.min).unwrap_or(f64::NEG_INFINITY),
                    max: s.and_then(|s| s.max).unwrap_or(f64::INFINITY),
                    vary: s.and_then(|s| s.vary).unwrap_or(true),
                    expr: s.map(|s| s.expr.clone()).unwrap_or_default(),
                }
            })
            .collect();
        Ok(Self {
            info: ModelInfo {
                name: spec.name.clone(),
                category: if spec.category.is_empty() {
                    "Expression".into()
                } else {
                    spec.category.clone()
                },
                description: if spec.description.is_empty() {
                    format!("y = {}", spec.formula)
                } else {
                    format!("{}\ny = {}", spec.description, spec.formula)
                },
                params,
            },
            compiled,
        })
    }

    pub fn load_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut spec = ExprModelSpec::from_toml(&text)?;
        if spec.name.is_empty()
            && let Some(stem) = path.file_stem()
        {
            spec.name = stem.to_string_lossy().into_owned();
        }
        Self::new(&spec)
    }
}

impl Model for ExprModel {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
        let mut vars = Vec::with_capacity(p.len() + 1);
        vars.push(0.0);
        vars.extend_from_slice(p);
        for (o, &xv) in out.iter_mut().zip(x) {
            vars[0] = xv;
            *o = self.compiled.eval(&vars);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_and_evaluates() {
        let spec = ExprModelSpec::from_toml(
            r#"
name = "decay"
formula = "a * exp(-x / tau) + c"
[[params]]
name = "tau"
default = 2.0
min = 0.0
"#,
        )
        .unwrap();
        let m = ExprModel::new(&spec).unwrap();
        let names: Vec<_> = m.info().params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "tau", "c"]);
        assert_eq!(m.info().params[1].default, 2.0);
        assert_eq!(m.info().params[1].min, 0.0);
        let mut out = [0.0; 2];
        m.eval(&[0.0, 2.0], &[3.0, 2.0, 1.0], &mut out).unwrap();
        assert_eq!(out[0], 4.0);
        assert!((out[1] - (3.0 * (-1.0f64).exp() + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn round_trips_through_toml() {
        let spec = ExprModelSpec {
            name: "n".into(),
            formula: "k * x".into(),
            params: vec![ExprParamSpec {
                name: "k".into(),
                default: Some(3.0),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(ExprModelSpec::from_toml(&spec.to_toml()).unwrap(), spec);
    }

    #[test]
    fn rejects_bad_formulas() {
        let spec = ExprModelSpec {
            name: "bad".into(),
            formula: "a * (x".into(),
            ..Default::default()
        };
        assert!(ExprModel::new(&spec).is_err());
    }
}
