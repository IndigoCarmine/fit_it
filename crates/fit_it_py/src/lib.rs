//! Embedded-Python bridge for fit_it.
//!
//! The app loads this library at runtime, and only when a `.py` model exists,
//! after preloading the user's Python DLL. Keeping pyo3 out of the main exe
//! means the app starts on machines without Python.
//!
//! The C ABI here is private to fit_it (see `src/plugin/python.rs` in the app);
//! the user-facing model format is documented there.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule, PyTuple};
use serde::Serialize;
use std::ffi::{CStr, CString, c_char};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const BRIDGE_ABI: u32 = 1;

const HOST: &CStr = cr#"
import sys, math
import numpy as _np

def describe(mod, stem):
    inf = math.inf
    name = str(getattr(mod, "name", stem))
    desc = str(getattr(mod, "description", "") or getattr(mod, "title", ""))
    cat = str(getattr(mod, "category", "Python"))
    params = []
    for e in getattr(mod, "parameters", []):
        e = list(e)
        if len(e) >= 3 and isinstance(e[1], str):
            n, unit, default = e[0], e[1], e[2]
            lim = e[3] if len(e) > 3 and e[3] is not None else [-inf, inf]
            lo, hi = lim[0], lim[1]
            ptype = e[4] if len(e) > 4 else ""
            d = e[5] if len(e) > 5 else ""
        else:
            n, default = e[0], e[1]
            lo = e[2] if len(e) > 2 else -inf
            hi = e[3] if len(e) > 3 else inf
            unit = ptype = d = ""
        params.append((str(n), str(unit), str(d), float(default), float(lo), float(hi), str(ptype or "")))
    # fit_it extension: param_defaults = {"c_tot": {"vary": False, "expr": "conc_M"}}
    extra = getattr(mod, "param_defaults", {}) or {}
    params = [p + (bool(extra.get(p[0], {}).get("vary", True)), str(extra.get(p[0], {}).get("expr", ""))) for p in params]
    if callable(getattr(mod, "Iq", None)):
        kind = "Iq"
    elif callable(getattr(mod, "f", None)):
        kind = "f"
    elif callable(getattr(mod, "model", None)):
        kind = "model"
    else:
        raise ValueError("define Iq(q, ...) (SasView style) or f(x, ...)")
    return name, desc, cat, params, kind, callable(getattr(mod, "guess", None)), callable(getattr(mod, "form_volume", None))

def call(f, xb, n, params):
    x = _np.frombuffer(xb, dtype=_np.float64, count=n)
    y = _np.asarray(f(x, *params), dtype=_np.float64)
    if y.shape != (n,):
        if y.ndim == 0:
            y = _np.full(n, float(y))
        else:
            y = _np.asarray(_np.vectorize(f)(x, *params), dtype=_np.float64).reshape(n)
    return _np.ascontiguousarray(y, dtype=_np.float64).tobytes()

def call_sas(f, xb, n, params, scale, background, vol_f, vol_params):
    y = _np.frombuffer(call(f, xb, n, params), dtype=_np.float64)
    if vol_f is not None:
        v = float(vol_f(*vol_params))
        if v != 0.0:
            y = y / v
    return _np.ascontiguousarray(scale * y + background, dtype=_np.float64).tobytes()

def guess(g, xb, yb, n):
    x = _np.frombuffer(xb, dtype=_np.float64, count=n)
    y = _np.frombuffer(yb, dtype=_np.float64, count=n)
    return [float(v) for v in g(x, y)]
"#;

static HOST_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[derive(Serialize)]
struct ParamMeta {
    name: String,
    unit: String,
    description: String,
    default: f64,
    /// `None` = unbounded (JSON has no infinity).
    min: Option<f64>,
    max: Option<f64>,
    vary: bool,
    expr: String,
}

#[derive(Serialize)]
struct Meta {
    name: String,
    category: String,
    description: String,
    params: Vec<ParamMeta>,
}

enum Kind {
    Plain,
    /// SasView `Iq`: parameters 0 and 1 are the implicit scale and background.
    Sas {
        volume: Option<(Py<PyAny>, Vec<usize>)>,
    },
}

pub struct PyModel {
    func: Py<PyAny>,
    guess: Option<Py<PyAny>>,
    kind: Kind,
    n_params: usize,
}

fn put_string(dst: *mut *mut c_char, s: String) {
    if !dst.is_null() {
        let s = CString::new(s.replace('\0', " ")).unwrap_or_default();
        unsafe { *dst = s.into_raw() };
    }
}

fn py_err(py: Python<'_>, e: PyErr) -> String {
    let tb = e
        .traceback(py)
        .and_then(|t| t.format().ok())
        .map(|t| format!("\n{t}"))
        .unwrap_or_default();
    format!("{e}{tb}")
}

fn to_bytes(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

fn host(py: Python<'_>) -> PyResult<&Bound<'_, PyModule>> {
    HOST_MODULE
        .get()
        .map(|m| m.bind(py))
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("bridge not initialised"))
}

#[unsafe(no_mangle)]
pub extern "C" fn fit_it_py_abi() -> u32 {
    BRIDGE_ABI
}

/// Start the interpreter. `sys_path_json` is a JSON list of strings for `sys.path`
/// (from probing the user's interpreter). On success `*info` receives the numpy
/// version, on failure the error text.
///
/// # Safety
/// Pointers must be valid; strings NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_init(
    sys_path_json: *const c_char,
    info: *mut *mut c_char,
) -> i32 {
    let json = unsafe { CStr::from_ptr(sys_path_json) }.to_string_lossy();
    let path: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
    Python::initialize();
    let r = Python::attach(|py| -> Result<String, String> {
        let run = || -> PyResult<String> {
            let sys = py.import("sys")?;
            if !path.is_empty() {
                sys.setattr(
                    "path",
                    path.iter().filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                )?;
            }
            if HOST_MODULE.get().is_none() {
                let m = PyModule::from_code(py, HOST, c"fit_it_host.py", c"fit_it_host")?;
                let _ = HOST_MODULE.set(m.unbind());
            }
            py.import("numpy")?.getattr("__version__")?.extract()
        };
        run().map_err(|e| py_err(py, e))
    });
    match r {
        Ok(v) => {
            put_string(info, v);
            0
        }
        Err(e) => {
            put_string(info, e);
            1
        }
    }
}

type Row = (String, String, String, f64, f64, f64, String, bool, String);

fn load(py: Python<'_>, path: &str) -> PyResult<(PyModel, Meta)> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let p = std::path::Path::new(path);
    let code = std::fs::read_to_string(p)
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();
    // Let the file import helpers that sit next to it.
    if let Some(dir) = p.parent().and_then(|d| d.to_str()) {
        let sys_path = py.import("sys")?.getattr("path")?;
        if !sys_path.contains(dir)? {
            sys_path.call_method1("append", (dir,))?;
        }
    }
    let module_name = format!(
        "fit_it_model_{stem}_{}",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let module = PyModule::from_code(
        py,
        &CString::new(code)?,
        &CString::new(path)?,
        &CString::new(module_name)?,
    )?;
    #[allow(clippy::type_complexity)]
    let (name, description, category, rows, kind, has_guess, has_volume): (
        String,
        String,
        String,
        Vec<Row>,
        String,
        bool,
        bool,
    ) = host(py)?
        .call_method1("describe", (&module, &stem))?
        .extract()?;
    let bound = |v: f64| v.is_finite().then_some(v);
    let mut params: Vec<ParamMeta> = rows
        .iter()
        .map(|(n, unit, desc, def, lo, hi, _, vary, expr)| ParamMeta {
            name: n.clone(),
            unit: unit.clone(),
            description: desc.clone(),
            default: *def,
            min: bound(*lo),
            max: bound(*hi),
            vary: *vary,
            expr: expr.clone(),
        })
        .collect();
    let func = module.getattr(kind.as_str())?.unbind();
    let kind = if kind == "Iq" {
        // SasView's implicit parameters, in SasView's order.
        let implicit = |name: &str, default: f64, min: Option<f64>| ParamMeta {
            name: name.into(),
            unit: String::new(),
            description: String::new(),
            default,
            min,
            max: None,
            vary: true,
            expr: String::new(),
        };
        params.insert(0, implicit("background", 0.001, None));
        params.insert(0, implicit("scale", 1.0, Some(0.0)));
        let volume = if has_volume {
            let idx = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.6 == "volume")
                .map(|(i, _)| i)
                .collect();
            Some((module.getattr("form_volume")?.unbind(), idx))
        } else {
            None
        };
        Kind::Sas { volume }
    } else {
        Kind::Plain
    };
    let model = PyModel {
        func,
        guess: if has_guess {
            Some(module.getattr("guess")?.unbind())
        } else {
            None
        },
        kind,
        n_params: params.len(),
    };
    Ok((
        model,
        Meta {
            name,
            category,
            description,
            params,
        },
    ))
}

/// Load a model file. Returns a handle, or null with the error in `*meta`.
/// On success `*meta` receives the model description as JSON.
///
/// # Safety
/// `path` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_load(
    path: *const c_char,
    meta: *mut *mut c_char,
) -> *mut PyModel {
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    Python::attach(|py| match load(py, &path) {
        Ok((model, m)) => {
            put_string(meta, serde_json::to_string(&m).unwrap_or_default());
            Box::into_raw(Box::new(model))
        }
        Err(e) => {
            put_string(meta, py_err(py, e));
            std::ptr::null_mut()
        }
    })
}

fn eval(py: Python<'_>, m: &PyModel, x: &[f64], p: &[f64], out: &mut [f64]) -> PyResult<()> {
    let host = host(py)?;
    let xb = PyBytes::new(py, &to_bytes(x));
    let res = match &m.kind {
        Kind::Plain => {
            host.call_method1("call", (m.func.bind(py), xb, x.len(), PyTuple::new(py, p)?))?
        }
        Kind::Sas { volume } => {
            let rest = &p[2..];
            let (vf, vp) = match volume {
                Some((f, idx)) => (
                    Some(f.bind(py).clone()),
                    idx.iter().map(|&i| rest[i]).collect::<Vec<f64>>(),
                ),
                None => (None, Vec::new()),
            };
            host.call_method1(
                "call_sas",
                (
                    m.func.bind(py),
                    xb,
                    x.len(),
                    PyTuple::new(py, rest)?,
                    p[0],
                    p[1],
                    vf,
                    PyTuple::new(py, vp)?,
                ),
            )?
        }
    };
    let bytes = res.downcast::<PyBytes>()?.as_bytes();
    for (o, chunk) in out.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *o = f64::from_ne_bytes(*chunk);
    }
    Ok(())
}

/// # Safety
/// `m` from `fit_it_py_load`; arrays of the stated lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_eval(
    m: *const PyModel,
    x: *const f64,
    n: usize,
    p: *const f64,
    np: usize,
    out: *mut f64,
    err: *mut *mut c_char,
) -> i32 {
    let m = unsafe { &*m };
    if np != m.n_params {
        put_string(err, "parameter count mismatch".into());
        return 1;
    }
    let (x, p, out) = unsafe {
        (
            std::slice::from_raw_parts(x, n),
            std::slice::from_raw_parts(p, np),
            std::slice::from_raw_parts_mut(out, n),
        )
    };
    Python::attach(|py| match eval(py, m, x, p, out) {
        Ok(()) => 0,
        Err(e) => {
            put_string(err, py_err(py, e));
            1
        }
    })
}

/// # Safety
/// `m` from `fit_it_py_load`; arrays of the stated lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_guess(
    m: *const PyModel,
    x: *const f64,
    y: *const f64,
    n: usize,
    p_out: *mut f64,
    np: usize,
) -> i32 {
    let m = unsafe { &*m };
    let Some(g) = &m.guess else { return 1 };
    let (x, y, out) = unsafe {
        (
            std::slice::from_raw_parts(x, n),
            std::slice::from_raw_parts(y, n),
            std::slice::from_raw_parts_mut(p_out, np),
        )
    };
    let v: Option<Vec<f64>> = Python::attach(|py| {
        host(py)
            .and_then(|h| {
                h.call_method1(
                    "guess",
                    (
                        g.bind(py),
                        PyBytes::new(py, &to_bytes(x)),
                        PyBytes::new(py, &to_bytes(y)),
                        n,
                    ),
                )
            })
            .and_then(|r| r.extract())
            .ok()
    });
    let Some(mut v) = v else { return 1 };
    if matches!(m.kind, Kind::Sas { .. }) {
        v.splice(0..0, [1.0, 0.0]);
    }
    if v.len() != np {
        return 1;
    }
    out.copy_from_slice(&v);
    0
}

/// # Safety
/// `m` from `fit_it_py_load`, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_release(m: *mut PyModel) {
    if !m.is_null() {
        let m = unsafe { Box::from_raw(m) };
        Python::attach(|_| drop(m));
    }
}

/// # Safety
/// `s` from this library, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fit_it_py_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
