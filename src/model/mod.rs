//! What a fit model is, independent of where it came from.
//!
//! Presets and user plugins are the same thing here: every model, whether it was
//! loaded from a native library, compiled from C/Rust source, run through the
//! embedded Python interpreter, or read from a `.fexpr` formula file, ends up as an
//! `Arc<dyn Model>` in the [`crate::plugin::Registry`].

pub mod composite;
pub mod expression;

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub name: String,
    pub unit: String,
    pub description: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    /// Fitted by default. Known inputs such as a sample concentration are not.
    pub vary: bool,
    /// Default constraint, applied when a dataset can resolve every name in it
    /// (e.g. `conc_M`, a constant read from the file name).
    pub expr: String,
}

impl ParamDef {
    pub fn new(name: impl Into<String>, default: f64) -> Self {
        Self {
            name: name.into(),
            unit: String::new(),
            description: String::new(),
            default,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            vary: true,
            expr: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelInfo {
    pub name: String,
    pub category: String,
    pub description: String,
    pub params: Vec<ParamDef>,
}

pub trait Model: Send + Sync {
    fn info(&self) -> &ModelInfo;

    /// Evaluate at every `x`, writing into `out` (same length as `x`).
    /// `p` holds the parameter values in [`ModelInfo::params`] order.
    fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String>;

    /// Initial parameter values estimated from data, when the model knows how.
    fn guess(&self, _x: &[f64], _y: &[f64]) -> Option<Vec<f64>> {
        None
    }
}

pub type ModelRef = Arc<dyn Model>;

/// Is `s` usable as a component or parameter name inside formulas?
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
pub(crate) mod testing {
    //! Tiny in-crate models so the fitting core can be tested without plugin files.
    use super::*;

    pub struct Gaussian(ModelInfo);

    impl Gaussian {
        pub fn arc() -> ModelRef {
            Arc::new(Gaussian(ModelInfo {
                name: "gaussian".into(),
                category: "Peak".into(),
                description: String::new(),
                params: vec![
                    ParamDef::new("amplitude", 1.0),
                    ParamDef::new("center", 0.0),
                    ParamDef {
                        min: 0.0,
                        ..ParamDef::new("sigma", 1.0)
                    },
                ],
            }))
        }
    }

    impl Model for Gaussian {
        fn info(&self) -> &ModelInfo {
            &self.0
        }
        fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
            let (a, c, s) = (p[0], p[1], p[2]);
            for (o, &x) in out.iter_mut().zip(x) {
                *o = a / (s * (2.0 * std::f64::consts::PI).sqrt())
                    * (-(x - c).powi(2) / (2.0 * s * s)).exp();
            }
            Ok(())
        }
    }

    pub struct Linear(ModelInfo);

    impl Linear {
        pub fn arc() -> ModelRef {
            Arc::new(Linear(ModelInfo {
                name: "linear".into(),
                category: "Background".into(),
                description: String::new(),
                params: vec![ParamDef::new("slope", 0.0), ParamDef::new("intercept", 0.0)],
            }))
        }
    }

    impl Model for Linear {
        fn info(&self) -> &ModelInfo {
            &self.0
        }
        fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
            for (o, &x) in out.iter_mut().zip(x) {
                *o = p[0] * x + p[1];
            }
            Ok(())
        }
    }

    pub fn lookup(name: &str) -> Option<ModelRef> {
        match name {
            "gaussian" => Some(Gaussian::arc()),
            "linear" => Some(Linear::arc()),
            _ => None,
        }
    }
}
