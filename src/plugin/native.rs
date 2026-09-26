//! Loading models from shared libraries that implement `fit_it_plugin.h`.

use crate::model::{Model, ModelInfo, ParamDef};
use libloading::Library;
use std::ffi::{CStr, c_char};
use std::path::Path;
use std::sync::Arc;

/// Newest ABI; version 1 libraries (no `flags`/`expr` per parameter) still load.
pub const ABI_VERSION: u32 = 2;
const PARAM_FIXED: u32 = 1;

#[repr(C)]
struct RawParamV1 {
    name: *const c_char,
    unit: *const c_char,
    description: *const c_char,
    default_value: f64,
    min: f64,
    max: f64,
}

#[repr(C)]
struct RawParam {
    v1: RawParamV1,
    flags: u32,
    expr: *const c_char,
}

type EvalFn = unsafe extern "C" fn(*const f64, usize, *const f64, *mut f64) -> i32;
type GuessFn = unsafe extern "C" fn(*const f64, *const f64, usize, *mut f64) -> i32;
type ModelsFn = unsafe extern "C" fn(*mut u32) -> *const RawModel;

#[repr(C)]
struct RawModel {
    abi_version: u32,
    n_params: u32,
    name: *const c_char,
    category: *const c_char,
    description: *const c_char,
    /// `RawParamV1` or `RawParam`, depending on `abi_version`.
    params: *const std::ffi::c_void,
    eval: Option<EvalFn>,
    guess: Option<GuessFn>,
}

pub struct NativeModel {
    info: ModelInfo,
    eval: EvalFn,
    guess: Option<GuessFn>,
    // Keeps the code behind the function pointers mapped.
    _lib: Arc<Library>,
}

/// # Safety
/// `p` must be null or a valid NUL-terminated string.
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

pub fn load(path: &Path) -> Result<Vec<NativeModel>, String> {
    // SAFETY: loading a library runs its initialisers; that is the point of a plugin.
    let lib = Arc::new(unsafe { Library::new(path) }.map_err(|e| e.to_string())?);
    let models_fn: ModelsFn = unsafe {
        *lib.get::<ModelsFn>(b"fit_it_models\0")
            .map_err(|_| "does not export `fit_it_models`".to_string())?
    };
    let mut count = 0u32;
    let raw = unsafe { models_fn(&mut count) };
    if raw.is_null() || count == 0 {
        return Err("`fit_it_models` returned no models".into());
    }
    let raw = unsafe { std::slice::from_raw_parts(raw, count as usize) };
    let mut out = Vec::new();
    for m in raw {
        if !(1..=ABI_VERSION).contains(&m.abi_version) {
            return Err(format!(
                "plugin ABI version {} (this app supports 1..={ABI_VERSION})",
                m.abi_version
            ));
        }
        let name = unsafe { text(m.name) };
        let eval = m.eval.ok_or_else(|| format!("{name}: `eval` is NULL"))?;
        let params = if m.n_params == 0 {
            Vec::new()
        } else {
            if m.params.is_null() {
                return Err(format!("{name}: `params` is NULL"));
            }
            let def = |p: &RawParamV1, flags: u32, expr: *const c_char| ParamDef {
                name: unsafe { text(p.name) },
                unit: unsafe { text(p.unit) },
                description: unsafe { text(p.description) },
                default: p.default_value,
                min: p.min,
                max: p.max,
                vary: flags & PARAM_FIXED == 0,
                expr: unsafe { text(expr) },
            };
            let n = m.n_params as usize;
            if m.abi_version == 1 {
                unsafe { std::slice::from_raw_parts(m.params as *const RawParamV1, n) }
                    .iter()
                    .map(|p| def(p, 0, std::ptr::null()))
                    .collect()
            } else {
                unsafe { std::slice::from_raw_parts(m.params as *const RawParam, n) }
                    .iter()
                    .map(|p| def(&p.v1, p.flags, p.expr))
                    .collect()
            }
        };
        out.push(NativeModel {
            info: ModelInfo {
                name,
                category: unsafe { text(m.category) },
                description: unsafe { text(m.description) },
                params,
            },
            eval,
            guess: m.guess,
            _lib: lib.clone(),
        });
    }
    Ok(out)
}

impl Model for NativeModel {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
        if p.len() != self.info.params.len() || out.len() != x.len() {
            return Err("parameter count mismatch".into());
        }
        // SAFETY: lengths checked above; the ABI contract covers the rest.
        let rc = unsafe { (self.eval)(x.as_ptr(), x.len(), p.as_ptr(), out.as_mut_ptr()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("eval returned error code {rc}"))
        }
    }

    fn guess(&self, x: &[f64], y: &[f64]) -> Option<Vec<f64>> {
        let g = self.guess?;
        let n = x.len().min(y.len());
        if n == 0 {
            return None;
        }
        let mut p: Vec<f64> = self.info.params.iter().map(|p| p.default).collect();
        let rc = unsafe { g(x.as_ptr(), y.as_ptr(), n, p.as_mut_ptr()) };
        (rc == 0).then_some(p)
    }
}
