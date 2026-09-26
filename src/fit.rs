//! Least-squares fitting: parameters, constraints, and a bounded Levenberg–Marquardt.
//!
//! A [`Problem`] holds one or more datasets, each with its own composite model and
//! parameters. Parameters are addressed globally as `<tag>.<name>`, so a constraint
//! like `D1.g1_sigma` ties datasets together — that is all a global fit is. Only
//! datasets marked active contribute residuals and free parameters; the others
//! still provide values for constraints.
//!
//! Bounds use the same transforms as lmfit/MINUIT, so the optimiser itself works
//! on unbounded internal variables.

use crate::data::FitArrays;
use crate::expr::{Compiled, Expr};
use crate::model::ParamDef;
use crate::model::composite::CompiledComposite;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// JSON has no infinity, so unbounded limits are stored as `null`.
mod bound {
    use serde::{Deserialize, Deserializer, Serializer};

    fn ser<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_some(v)
        } else {
            s.serialize_none()
        }
    }

    pub mod lower {
        use super::*;
        pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
            ser(v, s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
            Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NEG_INFINITY))
        }
    }

    pub mod upper {
        use super::*;
        pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
            ser(v, s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
            Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::INFINITY))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Param {
    pub name: String,
    pub value: f64,
    #[serde(with = "bound::lower")]
    pub min: f64,
    #[serde(with = "bound::upper")]
    pub max: f64,
    pub vary: bool,
    /// Constraint expression; when non-empty the value is computed, never fitted.
    pub expr: String,
    #[serde(skip)]
    pub stderr: Option<f64>,
}

impl Default for Param {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: 0.0,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            vary: true,
            expr: String::new(),
            stderr: None,
        }
    }
}

impl Param {
    pub fn from_def(d: &ParamDef) -> Self {
        Self {
            name: d.name.clone(),
            value: d.default,
            min: d.min,
            max: d.max,
            vary: d.vary,
            ..Default::default()
        }
    }
}

/// Make `params` match `defs`: keep existing entries by name, add new ones from
/// their defaults, drop the rest, and follow `defs` order. Returns the indices
/// of the newly added parameters.
pub fn sync_params(params: &mut Vec<Param>, defs: &[ParamDef]) -> Vec<usize> {
    let mut old = std::mem::take(params);
    let mut added = Vec::new();
    for d in defs {
        match old.iter().position(|p| p.name == d.name) {
            Some(i) => params.push(old.swap_remove(i)),
            None => {
                added.push(params.len());
                params.push(Param::from_def(d));
            }
        }
    }
    added
}

pub struct DatasetJob {
    pub tag: String,
    pub model: CompiledComposite,
    pub arrays: FitArrays,
    pub params: Vec<Param>,
    /// Contributes residuals and free parameters.
    pub active: bool,
    /// Fixed named values (sample concentration, ...) that constraints and
    /// derived quantities may use, as `name` or `<tag>.name`.
    pub constants: Vec<(String, f64)>,
}

pub struct Problem {
    datasets: Vec<DatasetJob>,
    /// Start of each dataset's parameters in the global vector; the last entry is
    /// where dataset constants begin.
    offsets: Vec<usize>,
    names: Vec<String>,
    init: Vec<f64>,
    bounds: Vec<(f64, f64)>,
    free: Vec<usize>,
    /// Constrained parameters in dependency order.
    exprs: Vec<(usize, Compiled)>,
}

fn compiled_vars(c: &Compiled, out: &mut Vec<usize>) {
    match c {
        Compiled::Num(_) => {}
        Compiled::Var(i) => out.push(*i),
        Compiled::Neg(a) => compiled_vars(a, out),
        Compiled::Bin(_, a, b) => {
            compiled_vars(a, out);
            compiled_vars(b, out);
        }
        Compiled::Call(_, args) => args.iter().for_each(|a| compiled_vars(a, out)),
    }
}

impl Problem {
    pub fn new(datasets: Vec<DatasetJob>) -> Result<Self, String> {
        let mut offsets = Vec::new();
        let mut names = Vec::new();
        let mut init = Vec::new();
        let mut bounds = Vec::new();
        let mut vary = Vec::new();
        let mut raw_exprs: Vec<(usize, usize, String)> = Vec::new();
        for (di, d) in datasets.iter().enumerate() {
            if datasets[..di].iter().any(|o| o.tag == d.tag) {
                return Err(format!("dataset tag `{}` is used twice", d.tag));
            }
            offsets.push(names.len());
            for def in d.model.params() {
                let p = d
                    .params
                    .iter()
                    .find(|p| p.name == def.name)
                    .cloned()
                    .unwrap_or_else(|| Param::from_def(def));
                let gi = names.len();
                names.push(format!("{}.{}", d.tag, p.name));
                let (lo, hi) = (p.min.min(p.max), p.max.max(p.min));
                init.push(if p.value.is_finite() {
                    p.value.clamp(lo, hi)
                } else {
                    def.default
                });
                bounds.push((lo, hi));
                vary.push(p.vary && d.active);
                if !p.expr.trim().is_empty() {
                    raw_exprs.push((gi, di, p.expr.clone()));
                }
            }
        }
        offsets.push(names.len());
        for d in &datasets {
            for (name, v) in &d.constants {
                let full = format!("{}.{name}", d.tag);
                if names.contains(&full) {
                    return Err(format!(
                        "constant `{full}` has the same name as a parameter"
                    ));
                }
                names.push(full);
                init.push(*v);
                bounds.push((f64::NEG_INFINITY, f64::INFINITY));
                vary.push(false);
            }
        }

        let mut compiled = Vec::new();
        for (gi, di, src) in &raw_exprs {
            let e = Expr::parse(src).map_err(|e| format!("{}: {e}", names[*gi]))?;
            let own = &datasets[*di].tag;
            let c = e
                .compile(&|n| {
                    let full = if n.contains('.') {
                        n.to_string()
                    } else {
                        format!("{own}.{n}")
                    };
                    names.iter().position(|m| *m == full)
                })
                .map_err(|e| format!("{}: {e}", names[*gi]))?;
            compiled.push((*gi, c));
        }

        // Order constraints so each one only reads values that are already final.
        let constrained: Vec<usize> = compiled.iter().map(|(g, _)| *g).collect();
        let mut done = vec![false; compiled.len()];
        let mut exprs = Vec::new();
        while exprs.len() < compiled.len() {
            let before = exprs.len();
            for (k, (gi, c)) in compiled.iter().enumerate() {
                if done[k] {
                    continue;
                }
                let mut deps = Vec::new();
                compiled_vars(c, &mut deps);
                let ready = deps
                    .iter()
                    .all(|d| match constrained.iter().position(|g| g == d) {
                        Some(j) => done[j],
                        None => true,
                    });
                if ready {
                    done[k] = true;
                    exprs.push((*gi, c.clone()));
                }
            }
            if exprs.len() == before {
                let stuck: Vec<&str> = compiled
                    .iter()
                    .zip(&done)
                    .filter(|(_, d)| !**d)
                    .map(|((g, _), _)| names[*g].as_str())
                    .collect();
                return Err(format!("circular constraints: {}", stuck.join(", ")));
            }
        }

        let free = (0..names.len())
            .filter(|&i| vary[i] && !constrained.contains(&i))
            .collect();
        Ok(Self {
            datasets,
            offsets,
            names,
            init,
            bounds,
            free,
            exprs,
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Number of model parameters (the names after them are dataset constants).
    pub fn n_params(&self) -> usize {
        *self.offsets.last().unwrap_or(&0)
    }

    /// Compile an expression over the global names. Bare names resolve against
    /// dataset `default_tag` when given.
    pub fn compile_expr(&self, src: &str, default_tag: Option<&str>) -> Result<Compiled, String> {
        let e = Expr::parse(src)?;
        e.compile(&|n| {
            self.names.iter().position(|m| m == n).or_else(|| {
                let tag = default_tag?;
                let full = format!("{tag}.{n}");
                self.names.iter().position(|m| *m == full)
            })
        })
    }

    pub fn free(&self) -> &[usize] {
        &self.free
    }

    pub fn datasets(&self) -> &[DatasetJob] {
        &self.datasets
    }

    /// Global values of dataset `d`'s parameters.
    pub fn dataset_values<'a>(&self, d: usize, values: &'a [f64]) -> &'a [f64] {
        &values[self.offsets[d]..self.offsets[d + 1]]
    }

    /// All parameter values given the free ones, with constraints applied.
    pub fn values_with(&self, free_vals: &[f64]) -> Vec<f64> {
        let mut v = self.init.clone();
        for (&i, &x) in self.free.iter().zip(free_vals) {
            v[i] = x;
        }
        for (i, c) in &self.exprs {
            v[*i] = c.eval(&v);
        }
        v
    }

    /// Current values (initial free values plus constraints).
    pub fn current_values(&self) -> Vec<f64> {
        let free: Vec<f64> = self.free.iter().map(|&i| self.init[i]).collect();
        self.values_with(&free)
    }

    pub fn ndata(&self) -> usize {
        self.datasets
            .iter()
            .filter(|d| d.active)
            .map(|d| d.arrays.x.len())
            .sum()
    }

    /// Weighted residuals `(y - f) / σ` of all active datasets, concatenated.
    pub fn residuals(&self, values: &[f64], out: &mut Vec<f64>) -> Result<(), String> {
        out.clear();
        for (di, d) in self.datasets.iter().enumerate() {
            if !d.active {
                continue;
            }
            let a = &d.arrays;
            let mut f = vec![0.0; a.x.len()];
            d.model
                .eval(&a.x, self.dataset_values(di, values), &mut f)
                .map_err(|e| format!("{}: {e}", d.tag))?;
            out.extend(f.iter().zip(&a.y).zip(&a.w).map(|((f, y), w)| (y - f) * w));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Bounds transforms (lmfit / MINUIT)

fn to_internal(v: f64, (lo, hi): (f64, f64)) -> f64 {
    match (lo.is_finite(), hi.is_finite()) {
        (true, true) => {
            if hi <= lo {
                return 0.0;
            }
            // Keep off the exact edge, where the transform's derivative vanishes.
            let t = (2.0 * (v - lo) / (hi - lo) - 1.0).clamp(-1.0 + 1e-9, 1.0 - 1e-9);
            t.asin()
        }
        (true, false) => ((v - lo + 1.0).max(1.0 + 1e-9).powi(2) - 1.0).sqrt(),
        (false, true) => ((hi - v + 1.0).max(1.0 + 1e-9).powi(2) - 1.0).sqrt(),
        (false, false) => v,
    }
}

fn to_external(u: f64, (lo, hi): (f64, f64)) -> f64 {
    match (lo.is_finite(), hi.is_finite()) {
        (true, true) => {
            if hi <= lo {
                lo
            } else {
                lo + (u.sin() + 1.0) * (hi - lo) / 2.0
            }
        }
        (true, false) => lo - 1.0 + (u * u + 1.0).sqrt(),
        (false, true) => hi + 1.0 - (u * u + 1.0).sqrt(),
        (false, false) => u,
    }
}

/// d(external)/d(internal)
fn ext_derivative(u: f64, (lo, hi): (f64, f64)) -> f64 {
    match (lo.is_finite(), hi.is_finite()) {
        (true, true) => u.cos() * (hi - lo) / 2.0,
        (true, false) => u / (u * u + 1.0).sqrt(),
        (false, true) => -u / (u * u + 1.0).sqrt(),
        (false, false) => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Dense linear algebra for small systems (row-major n×n).

fn cholesky_solve(a: &[f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..=i {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= l[i * n + k] * l[j * n + k];
            }
            if i == j {
                if s <= 0.0 || !s.is_finite() {
                    return None;
                }
                l[i * n + i] = s.sqrt();
            } else {
                l[i * n + j] = s / l[j * n + j];
            }
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let s: f64 = (0..i).map(|k| l[i * n + k] * y[k]).sum();
        y[i] = (b[i] - s) / l[i * n + i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|k| l[k * n + i] * x[k]).sum();
        x[i] = (y[i] - s) / l[i * n + i];
    }
    Some(x)
}

fn invert(a: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut m = a.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs()));
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| m[i * n + c].abs().total_cmp(&m[j * n + c].abs()))?;
        if m[p * n + c].abs() <= scale * 1e-14 || !m[p * n + c].is_finite() {
            return None;
        }
        for k in 0..n {
            m.swap(c * n + k, p * n + k);
            inv.swap(c * n + k, p * n + k);
        }
        let d = m[c * n + c];
        for k in 0..n {
            m[c * n + k] /= d;
            inv[c * n + k] /= d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * n + c];
                if f != 0.0 {
                    for k in 0..n {
                        m[r * n + k] -= f * m[c * n + k];
                        inv[r * n + k] -= f * inv[c * n + k];
                    }
                }
            }
        }
    }
    Some(inv)
}

// ---------------------------------------------------------------------------
// Levenberg–Marquardt

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FitOptions {
    pub max_nfev: usize,
    pub ftol: f64,
    pub xtol: f64,
    pub gtol: f64,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            max_nfev: 4000,
            ftol: 1e-10,
            xtol: 1e-10,
            gtol: 1e-10,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub nfev: usize,
    pub iter: usize,
    pub chisqr: f64,
}

#[derive(Clone, Debug, Default)]
pub struct FitOutcome {
    pub success: bool,
    pub message: String,
    pub nfev: usize,
    pub niter: usize,
    pub ndata: usize,
    pub nvarys: usize,
    pub chisqr: f64,
    pub redchi: f64,
    pub aic: f64,
    pub bic: f64,
    pub names: Vec<String>,
    pub values: Vec<f64>,
    pub init: Vec<f64>,
    pub stderr: Vec<Option<f64>>,
    pub free: Vec<usize>,
    pub constrained: Vec<(usize, String)>,
    /// Correlations between free parameters, |r| sorted descending.
    pub correl: Vec<(usize, usize, f64)>,
    /// Unweighted R² per active dataset.
    pub r2: Vec<(String, f64)>,
    /// Model parameters come first in `names`; dataset constants follow.
    pub n_params: usize,
    /// Covariance of the free parameters (row-major, `free.len()²`), when available.
    pub cov: Option<Vec<f64>>,
    /// Quantities computed from the result, filled by [`derive`].
    pub derived: Vec<Derived>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    pub name: String,
    pub expr: String,
    pub value: Result<f64, String>,
    pub stderr: Option<f64>,
}

/// Evaluate derived quantities (e.g. `D1.t1_deltaH - 300 * D1.t1_deltaS`) at the
/// fitted values, with uncertainties propagated from the covariance through a
/// numerical gradient. Bare names resolve against `default_tag`.
pub fn derive(
    problem: &Problem,
    out: &FitOutcome,
    specs: &[(String, String)],
    default_tag: Option<&str>,
) -> Vec<Derived> {
    let n = out.free.len();
    specs
        .iter()
        .map(|(name, expr)| {
            let mut d = Derived {
                name: name.clone(),
                expr: expr.clone(),
                value: Err(String::new()),
                stderr: None,
            };
            let c = match problem.compile_expr(expr, default_tag) {
                Ok(c) => c,
                Err(e) => {
                    d.value = Err(e);
                    return d;
                }
            };
            let v0 = c.eval(&out.values);
            d.value = Ok(v0);
            if let Some(cov) = &out.cov {
                let free_ext: Vec<f64> = out.free.iter().map(|&i| out.values[i]).collect();
                let g: Vec<f64> = (0..n)
                    .map(|j| {
                        let h = 1e-7 * free_ext[j].abs().max(1e-7);
                        let mut fp = free_ext.clone();
                        fp[j] += h;
                        (c.eval(&problem.values_with(&fp)) - v0) / h
                    })
                    .collect();
                let var: f64 = (0..n)
                    .flat_map(|j| (0..n).map(move |k| (j, k)))
                    .map(|(j, k)| g[j] * cov[j * n + k] * g[k])
                    .sum();
                if var.is_finite() && var >= 0.0 {
                    d.stderr = Some(var.sqrt());
                }
            }
            d
        })
        .collect()
}

struct Evaluator<'a> {
    problem: &'a Problem,
    nfev: usize,
    buf: Vec<f64>,
}

impl Evaluator<'_> {
    fn ext(&self, u: &[f64]) -> Vec<f64> {
        let free: Vec<f64> = u
            .iter()
            .zip(&self.problem.free)
            .map(|(&u, &i)| to_external(u, self.problem.bounds[i]))
            .collect();
        self.problem.values_with(&free)
    }

    /// Residuals at internal point `u`; `None` when the model fails or goes non-finite.
    fn residuals(&mut self, u: &[f64]) -> Option<Vec<f64>> {
        self.nfev += 1;
        let values = self.ext(u);
        let mut buf = std::mem::take(&mut self.buf);
        let ok =
            self.problem.residuals(&values, &mut buf).is_ok() && buf.iter().all(|r| r.is_finite());
        let out = ok.then(|| buf.clone());
        self.buf = buf;
        out
    }

    /// Forward-difference Jacobian of the residuals (m×n, row-major).
    fn jacobian(&mut self, u: &[f64], r0: &[f64]) -> Option<Vec<f64>> {
        let (m, n) = (r0.len(), u.len());
        let mut jac = vec![0.0; m * n];
        let mut up = u.to_vec();
        for j in 0..n {
            let h = if u[j] == 0.0 {
                1.49e-8
            } else {
                1.49e-8 * u[j].abs()
            };
            up[j] = u[j] + h;
            let r = self.residuals(&up)?;
            up[j] = u[j];
            for i in 0..m {
                jac[i * n + j] = (r[i] - r0[i]) / h;
            }
        }
        Some(jac)
    }
}

fn normal_equations(jac: &[f64], r: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let m = r.len();
    let mut a = vec![0.0; n * n];
    let mut g = vec![0.0; n];
    for i in 0..m {
        let row = &jac[i * n..(i + 1) * n];
        for j in 0..n {
            g[j] += row[j] * r[i];
            for k in 0..=j {
                a[j * n + k] += row[j] * row[k];
            }
        }
    }
    for j in 0..n {
        for k in 0..j {
            a[k * n + j] = a[j * n + k];
        }
    }
    (a, g)
}

pub fn fit(
    problem: &Problem,
    opts: &FitOptions,
    progress: Option<&Mutex<Progress>>,
    cancel: Option<&AtomicBool>,
) -> Result<FitOutcome, String> {
    let n = problem.free.len();
    let ndata = problem.ndata();
    if ndata == 0 {
        return Err("no data points in the fit range".into());
    }
    if ndata < n {
        return Err(format!(
            "{ndata} data points cannot determine {n} free parameters"
        ));
    }
    let mut ev = Evaluator {
        problem,
        nfev: 0,
        buf: Vec::new(),
    };
    let init = problem.current_values();
    let mut u: Vec<f64> = problem
        .free
        .iter()
        .map(|&i| to_internal(init[i], problem.bounds[i]))
        .collect();
    let mut r = match ev.residuals(&u) {
        Some(r) => r,
        None => {
            let mut buf = Vec::new();
            problem.residuals(&init, &mut buf)?;
            return Err("the model returns NaN or infinity at the initial parameters".into());
        }
    };
    let mut cost: f64 = r.iter().map(|v| v * v).sum();
    let report = |iter: usize, nfev: usize, chisqr: f64| {
        if let Some(p) = progress
            && let Ok(mut p) = p.lock()
        {
            *p = Progress { nfev, iter, chisqr };
        }
    };

    let mut lambda = 1e-3;
    let mut diag = vec![0.0f64; n];
    let mut niter = 0;
    let (mut success, mut message) = (true, String::from("no free parameters"));
    if n > 0 {
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                success = false;
                message = "cancelled".into();
                break;
            }
            if ev.nfev >= opts.max_nfev {
                success = false;
                message = format!("stopped after {} function evaluations (max_nfev)", ev.nfev);
                break;
            }
            niter += 1;
            report(niter, ev.nfev, cost);
            let Some(jac) = ev.jacobian(&u, &r) else {
                success = false;
                message = "the model failed while computing derivatives".into();
                break;
            };
            let (a, g) = normal_equations(&jac, &r, n);
            for j in 0..n {
                diag[j] = diag[j].max(a[j * n + j]);
                if diag[j] == 0.0 {
                    diag[j] = 1.0;
                }
            }
            let gnorm = (0..n)
                .map(|j| g[j].abs() / (a[j * n + j].sqrt() * cost.sqrt()).max(f64::MIN_POSITIVE))
                .fold(0.0, f64::max);
            if gnorm <= opts.gtol || cost == 0.0 {
                message = "converged: gradient is orthogonal to the residuals (gtol)".into();
                break;
            }
            let mut improved = false;
            let mut stop = None;
            while ev.nfev < opts.max_nfev {
                let mut m = a.clone();
                for j in 0..n {
                    m[j * n + j] += lambda * diag[j];
                }
                let neg_g: Vec<f64> = g.iter().map(|v| -v).collect();
                let Some(step) = cholesky_solve(&m, &neg_g, n) else {
                    lambda *= 10.0;
                    if lambda > 1e16 {
                        stop = Some("the normal equations are singular".to_string());
                        break;
                    }
                    continue;
                };
                // The residuals are (y - f), so the Gauss-Newton step is -(JᵀJ)⁻¹Jᵀr with J = ∂r/∂u.
                let trial: Vec<f64> = u.iter().zip(&step).map(|(u, s)| u + s).collect();
                let new_r = ev.residuals(&trial);
                let new_cost = new_r
                    .as_ref()
                    .map_or(f64::INFINITY, |r| r.iter().map(|v| v * v).sum());
                if new_cost < cost {
                    let rel = (cost - new_cost) / cost;
                    let step_norm = step.iter().map(|s| s * s).sum::<f64>().sqrt();
                    let u_norm = u.iter().map(|s| s * s).sum::<f64>().sqrt();
                    u = trial;
                    r = new_r.unwrap();
                    cost = new_cost;
                    lambda = (lambda / 10.0).max(1e-15);
                    improved = true;
                    if rel <= opts.ftol {
                        stop = Some("converged: relative χ² change below ftol".into());
                    } else if step_norm <= opts.xtol * (u_norm + opts.xtol) {
                        stop = Some("converged: parameter change below xtol".into());
                    }
                    break;
                }
                lambda *= 10.0;
                if lambda > 1e16 {
                    stop = Some("converged: no further improvement possible".into());
                    break;
                }
            }
            if let Some(s) = stop {
                message = s;
                break;
            }
            if !improved {
                success = false;
                message = format!("stopped after {} function evaluations (max_nfev)", ev.nfev);
                break;
            }
        }
    }
    report(niter, ev.nfev, cost);

    // Statistics
    let values = ev.ext(&u);
    let nfree = ndata.saturating_sub(n);
    let redchi = if nfree > 0 {
        cost / nfree as f64
    } else {
        f64::NAN
    };
    let nd = ndata as f64;
    let chi_per = (cost / nd).max(f64::MIN_POSITIVE);
    let aic = nd * chi_per.ln() + 2.0 * n as f64;
    let bic = nd * chi_per.ln() + nd.ln() * n as f64;

    // Covariance of the free external parameters: D (JᵀJ)⁻¹ D · χ²ᵣ.
    let mut stderr = vec![None; values.len()];
    let mut correl = Vec::new();
    let mut covariance = None;
    if n > 0
        && nfree > 0
        && let Some(jac) = ev.jacobian(&u, &r)
    {
        let (a, _) = normal_equations(&jac, &r, n);
        if let Some(inv) = invert(&a, n) {
            let d: Vec<f64> = u
                .iter()
                .zip(&problem.free)
                .map(|(&u, &i)| ext_derivative(u, problem.bounds[i]))
                .collect();
            let cov: Vec<f64> = (0..n * n)
                .map(|k| inv[k] * d[k / n] * d[k % n] * redchi)
                .collect();
            for j in 0..n {
                let v = cov[j * n + j];
                if v.is_finite() && v >= 0.0 {
                    stderr[problem.free[j]] = Some(v.sqrt());
                }
            }
            for j in 0..n {
                for k in 0..j {
                    let den = (cov[j * n + j] * cov[k * n + k]).sqrt();
                    if den > 0.0 {
                        correl.push((problem.free[k], problem.free[j], cov[j * n + k] / den));
                    }
                }
            }
            correl.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()));
            covariance = Some(cov.clone());

            // Propagate to constrained parameters through a numerical gradient.
            if !problem.exprs.is_empty() {
                let free_ext: Vec<f64> = problem.free.iter().map(|&i| values[i]).collect();
                let mut grads = vec![vec![0.0; n]; values.len()];
                for j in 0..n {
                    let h = 1e-7 * free_ext[j].abs().max(1e-7);
                    let mut fp = free_ext.clone();
                    fp[j] += h;
                    let vp = problem.values_with(&fp);
                    for (gi, _) in &problem.exprs {
                        grads[*gi][j] = (vp[*gi] - values[*gi]) / h;
                    }
                }
                for (gi, _) in &problem.exprs {
                    let g = &grads[*gi];
                    let var: f64 = (0..n)
                        .flat_map(|j| (0..n).map(move |k| (j, k)))
                        .map(|(j, k)| g[j] * cov[j * n + k] * g[k])
                        .sum();
                    if var.is_finite() && var >= 0.0 {
                        stderr[*gi] = Some(var.sqrt());
                    }
                }
            }
        }
    }

    let mut r2 = Vec::new();
    for (di, d) in problem.datasets.iter().enumerate() {
        if !d.active || d.arrays.x.is_empty() {
            continue;
        }
        let a = &d.arrays;
        let mut f = vec![0.0; a.x.len()];
        if d.model
            .eval(&a.x, problem.dataset_values(di, &values), &mut f)
            .is_ok()
        {
            let mean = a.y.iter().sum::<f64>() / a.y.len() as f64;
            let ss_res: f64 = a.y.iter().zip(&f).map(|(y, f)| (y - f).powi(2)).sum();
            let ss_tot: f64 = a.y.iter().map(|y| (y - mean).powi(2)).sum();
            r2.push((
                d.tag.clone(),
                if ss_tot > 0.0 {
                    1.0 - ss_res / ss_tot
                } else {
                    f64::NAN
                },
            ));
        }
    }

    let constrained = problem
        .exprs
        .iter()
        .map(|(gi, _)| {
            let d = problem.offsets.iter().rposition(|&o| o <= *gi).unwrap_or(0);
            let local = &problem.names[*gi][problem.datasets[d].tag.len() + 1..];
            let src = problem.datasets[d]
                .params
                .iter()
                .find(|p| p.name == local)
                .map(|p| p.expr.clone())
                .unwrap_or_default();
            (*gi, src)
        })
        .collect();

    Ok(FitOutcome {
        success,
        message,
        nfev: ev.nfev,
        niter,
        ndata,
        nvarys: n,
        chisqr: cost,
        redchi,
        aic,
        bic,
        names: problem.names.clone(),
        values,
        init,
        stderr,
        free: problem.free.clone(),
        constrained,
        correl,
        r2,
        n_params: problem.n_params(),
        cov: covariance,
        derived: Vec::new(),
    })
}

fn fmt_num(v: f64) -> String {
    if v == 0.0 || (1e-3..1e5).contains(&v.abs()) {
        format!("{v:.6}")
    } else {
        format!("{v:.6e}")
    }
}

impl FitOutcome {
    /// Values of a dataset's parameters by local name, from this outcome.
    pub fn value_of(&self, full_name: &str) -> Option<(f64, Option<f64>)> {
        let i = self.names.iter().position(|n| n == full_name)?;
        Some((self.values[i], self.stderr[i]))
    }

    /// An lmfit-style text report.
    pub fn report(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "[[Fit Statistics]]");
        let _ = writeln!(
            s,
            "    status             = {}",
            if self.success {
                "success"
            } else {
                "not converged"
            }
        );
        let _ = writeln!(s, "    message            = {}", self.message);
        let _ = writeln!(s, "    # function evals   = {}", self.nfev);
        let _ = writeln!(s, "    # iterations       = {}", self.niter);
        let _ = writeln!(s, "    # data points      = {}", self.ndata);
        let _ = writeln!(s, "    # variables        = {}", self.nvarys);
        let _ = writeln!(s, "    chi-square         = {}", fmt_num(self.chisqr));
        let _ = writeln!(s, "    reduced chi-square = {}", fmt_num(self.redchi));
        let _ = writeln!(s, "    Akaike info crit   = {:.4}", self.aic);
        let _ = writeln!(s, "    Bayesian info crit = {:.4}", self.bic);
        for (tag, r2) in &self.r2 {
            let _ = writeln!(
                s,
                "    R-squared [{tag}]{}= {r2:.6}",
                " ".repeat(8usize.saturating_sub(tag.len()))
            );
        }
        let _ = writeln!(s, "[[Variables]]");
        let params = &self.names[..self.n_params.min(self.names.len())];
        let width = params.iter().map(|n| n.len()).max().unwrap_or(0);
        for (i, name) in params.iter().enumerate() {
            let v = self.values[i];
            let mut line = format!("    {name:width$}: {}", fmt_num(v));
            if let Some(e) = self.stderr[i] {
                let pct = if v != 0.0 {
                    format!(" ({:.2}%)", 100.0 * e / v.abs())
                } else {
                    String::new()
                };
                let _ = write!(line, " +/- {}{pct}", fmt_num(e));
            }
            if let Some((_, src)) = self.constrained.iter().find(|(g, _)| *g == i) {
                let _ = write!(line, " == '{src}'");
            } else if self.free.contains(&i) {
                let _ = write!(line, " (init = {})", fmt_num(self.init[i]));
            } else {
                let _ = write!(line, " (fixed)");
            }
            let _ = writeln!(s, "{line}");
        }
        let shown: Vec<_> = self.correl.iter().filter(|c| c.2.abs() >= 0.1).collect();
        if !shown.is_empty() {
            let _ = writeln!(s, "[[Correlations]] (unreported correlations are < 0.100)");
            for (a, b, c) in shown {
                let pair = format!("C({}, {})", self.names[*a], self.names[*b]);
                let _ = writeln!(s, "    {pair:w$} = {c:+.4}", w = 2 * width + 6);
            }
        }
        if !self.derived.is_empty() {
            let _ = writeln!(s, "[[Derived]]");
            let w = self.derived.iter().map(|d| d.name.len()).max().unwrap_or(0);
            for d in &self.derived {
                let mut line = format!("    {:w$}: ", d.name);
                match &d.value {
                    Ok(v) => {
                        line.push_str(&fmt_num(*v));
                        if let Some(e) = d.stderr {
                            let _ = write!(line, " +/- {}", fmt_num(e));
                        }
                    }
                    Err(e) => line.push_str(&format!("error: {e}")),
                }
                let _ = writeln!(s, "{line}  = {}", d.expr);
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::composite::{Component, ModelSpec};
    use crate::model::testing::lookup;

    fn composite(parts: &[(&str, &str)]) -> CompiledComposite {
        let spec = ModelSpec {
            components: parts
                .iter()
                .map(|(n, m)| Component {
                    name: n.to_string(),
                    model: m.to_string(),
                })
                .collect(),
            formula: String::new(),
        };
        CompiledComposite::build(&spec, &lookup).unwrap()
    }

    fn synth(model: &CompiledComposite, p: &[f64], noise: f64) -> FitArrays {
        let x: Vec<f64> = (0..201).map(|i| -10.0 + i as f64 * 0.1).collect();
        let mut y = vec![0.0; x.len()];
        model.eval(&x, p, &mut y).unwrap();
        // Deterministic pseudo-noise so the test is stable.
        for (i, v) in y.iter_mut().enumerate() {
            *v += noise * ((i as f64 * 12.9898).sin() * 43758.5453).fract();
        }
        FitArrays {
            w: vec![1.0; x.len()],
            x,
            y,
        }
    }

    fn params(model: &CompiledComposite, values: &[f64]) -> Vec<Param> {
        model
            .params()
            .iter()
            .zip(values)
            .map(|(d, &v)| Param {
                value: v,
                ..Param::from_def(d)
            })
            .collect()
    }

    #[test]
    fn fits_gaussian_on_linear_background() {
        let m = composite(&[("g1", "gaussian"), ("bg", "linear")]);
        let truth = [5.0, 1.5, 0.8, 0.1, 2.0];
        let arrays = synth(&m, &truth, 0.01);
        let job = DatasetJob {
            tag: "D1".into(),
            params: params(&m, &[3.0, 0.5, 1.5, 0.0, 1.0]),
            model: m,
            arrays,
            active: true,
            constants: Vec::new(),
        };
        let problem = Problem::new(vec![job]).unwrap();
        let out = fit(&problem, &FitOptions::default(), None, None).unwrap();
        assert!(out.success, "{}", out.message);
        for (v, t) in out.values.iter().zip(truth) {
            assert!((v - t).abs() < 0.02, "{v} vs {t}\n{}", out.report());
        }
        assert!(out.stderr.iter().all(Option::is_some));
        assert!(out.report().contains("D1.g1_center"));
    }

    #[test]
    fn respects_bounds_and_fixed_parameters() {
        let m = composite(&[("g1", "gaussian")]);
        let arrays = synth(&m, &[5.0, 1.5, 0.8], 0.0);
        let mut ps = params(&m, &[3.0, 0.0, 1.0]);
        ps[1].max = 1.0; // center cannot reach 1.5
        ps[2].vary = false;
        let problem = Problem::new(vec![DatasetJob {
            tag: "D1".into(),
            model: m,
            arrays,
            params: ps,
            active: true,
            constants: Vec::new(),
        }])
        .unwrap();
        let out = fit(&problem, &FitOptions::default(), None, None).unwrap();
        assert!(out.values[1] <= 1.0 + 1e-9);
        assert!(out.values[1] > 0.9);
        assert_eq!(out.values[2], 1.0);
        assert_eq!(out.nvarys, 2);
    }

    #[test]
    fn global_fit_shares_a_parameter_across_datasets() {
        let m = composite(&[("g1", "gaussian")]);
        let d1 = synth(&m, &[5.0, -1.0, 0.7], 0.005);
        let d2 = synth(&m, &[2.0, 2.0, 0.7], 0.005);
        let mut p2 = params(&m, &[1.0, 1.0, 1.0]);
        p2[2].expr = "D1.g1_sigma".into();
        let problem = Problem::new(vec![
            DatasetJob {
                tag: "D1".into(),
                model: m.clone(),
                arrays: d1,
                params: params(&m, &[4.0, -0.5, 1.2]),
                active: true,
                constants: Vec::new(),
            },
            DatasetJob {
                tag: "D2".into(),
                model: m,
                arrays: d2,
                params: p2,
                active: true,
                constants: Vec::new(),
            },
        ])
        .unwrap();
        assert_eq!(problem.free().len(), 5);
        let out = fit(&problem, &FitOptions::default(), None, None).unwrap();
        let (s1, _) = out.value_of("D1.g1_sigma").unwrap();
        let (s2, e2) = out.value_of("D2.g1_sigma").unwrap();
        assert!((s1 - 0.7).abs() < 0.01);
        assert_eq!(s1, s2);
        assert!(e2.is_some(), "constrained params get propagated errors");
        assert!((out.value_of("D2.g1_center").unwrap().0 - 2.0).abs() < 0.01);
    }

    #[test]
    fn detects_circular_and_unknown_constraints() {
        let m = composite(&[("g1", "gaussian")]);
        let mut ps = params(&m, &[1.0, 0.0, 1.0]);
        ps[0].expr = "g1_center".into();
        ps[1].expr = "2 * g1_amplitude".into();
        let job = |ps: Vec<Param>| DatasetJob {
            tag: "D1".into(),
            model: m.clone(),
            arrays: FitArrays::default(),
            params: ps,
            active: true,
            constants: Vec::new(),
        };
        let err = Problem::new(vec![job(ps)]).err().unwrap();
        assert!(err.contains("circular"), "{err}");
        let mut ps = params(&m, &[1.0, 0.0, 1.0]);
        ps[0].expr = "D9.x".into();
        assert!(Problem::new(vec![job(ps)]).is_err());
    }

    #[test]
    fn inactive_datasets_are_fixed_but_still_feed_constraints() {
        let m = composite(&[("g1", "gaussian")]);
        let mut p2 = params(&m, &[1.0, 1.0, 1.0]);
        p2[2].expr = "D1.g1_sigma * 2".into();
        let problem = Problem::new(vec![
            DatasetJob {
                tag: "D1".into(),
                model: m.clone(),
                arrays: FitArrays::default(),
                params: params(&m, &[1.0, 0.0, 0.25]),
                active: false,
                constants: Vec::new(),
            },
            DatasetJob {
                tag: "D2".into(),
                model: m,
                arrays: FitArrays::default(),
                params: p2,
                active: true,
                constants: Vec::new(),
            },
        ])
        .unwrap();
        assert_eq!(problem.free().len(), 2);
        assert_eq!(problem.current_values()[5], 0.5);
    }

    #[test]
    fn constants_feed_constraints_and_derived_values() {
        let m = composite(&[("g1", "gaussian")]);
        let arrays = synth(&m, &[5.0, 1.5, 0.8], 0.01);
        let mut ps = params(&m, &[3.0, 1.0, 1.0]);
        // amplitude = 2 * area  (`area` is a dataset constant)
        ps[0].expr = "2 * area".into();
        let problem = Problem::new(vec![DatasetJob {
            tag: "D1".into(),
            model: m,
            arrays,
            params: ps,
            active: true,
            constants: vec![("area".into(), 2.5)],
        }])
        .unwrap();
        assert_eq!(problem.n_params(), 3);
        assert_eq!(problem.current_values()[0], 5.0);
        let mut out = fit(&problem, &FitOptions::default(), None, None).unwrap();
        assert!((out.values[1] - 1.5).abs() < 1e-3);
        let specs = vec![
            ("width".to_string(), "2.3548 * g1_sigma".to_string()),
            ("bad".to_string(), "nope + 1".to_string()),
        ];
        out.derived = derive(&problem, &out, &specs, Some("D1"));
        let w = &out.derived[0];
        assert!((w.value.clone().unwrap() - 2.3548 * out.values[2]).abs() < 1e-12);
        let e = w.stderr.unwrap();
        assert!((e - 2.3548 * out.stderr[2].unwrap()).abs() < 1e-6 * e.max(1e-12));
        assert!(out.derived[1].value.is_err());
        let report = out.report();
        assert!(report.contains("[[Derived]]") && report.contains("width"));
        assert!(!report.contains("D1.area"), "constants are not variables");
    }

    #[test]
    fn bound_transforms_round_trip() {
        for b in [
            (0.0, 10.0),
            (1.0, f64::INFINITY),
            (f64::NEG_INFINITY, -2.0),
            (f64::NEG_INFINITY, f64::INFINITY),
        ] {
            for v in [-3.0f64, 0.5, 4.0, 9.5] {
                let v = v.clamp(b.0, b.1);
                let back = to_external(to_internal(v, b), b);
                assert!((back - v).abs() < 1e-6, "{v} in {b:?} -> {back}");
            }
        }
    }

    #[test]
    fn params_serialize_infinite_bounds_as_null() {
        let p = Param {
            name: "a".into(),
            min: 0.0,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"max\":null"));
        let back: Param = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn sync_keeps_existing_values() {
        let defs = vec![ParamDef::new("a", 1.0), ParamDef::new("b", 2.0)];
        let mut ps = vec![
            Param {
                name: "b".into(),
                value: 7.0,
                ..Default::default()
            },
            Param {
                name: "gone".into(),
                ..Default::default()
            },
        ];
        sync_params(&mut ps, &defs);
        assert_eq!(
            ps.iter()
                .map(|p| (p.name.as_str(), p.value))
                .collect::<Vec<_>>(),
            [("a", 1.0), ("b", 7.0)]
        );
    }
}
