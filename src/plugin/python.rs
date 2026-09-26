//! Python model files (`*.py`), run in the user's own Python through a bridge.
//!
//! The format follows SasView plugin models: module-level `name`, `description`,
//! `category`, `parameters` and a vectorised function. Two function styles:
//!
//! - `Iq(q, *params)` — SasView style. `scale` and `background` are added
//!   automatically: `I = scale * Iq / form_volume(*volume_params) + background`
//!   (`form_volume` optional).
//! - `f(x, *params)` (or `model`) — plain function, used as is.
//!
//! `parameters` entries may be SasView rows
//! `[name, units, default, [lower, upper], type, description]` or short forms
//! `(name, default)` / `(name, default, lower, upper)`. An optional
//! `guess(x, y)` returns initial values for the declared parameters.
//!
//! How it runs: the configured interpreter is probed once as a subprocess (home,
//! version, `sys.path`, numpy); its shared library is preloaded; then the
//! pyo3-based bridge `fit_it_py` (a separate library next to the exe) is loaded
//! and embeds that interpreter. The exe itself never links Python, so it starts
//! on machines without Python and only `.py` models become unavailable.

use crate::model::{Model, ModelInfo, ParamDef};
use libloading::Library;
use serde::Deserialize;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const BRIDGE_ABI: u32 = 1;

type InitFn = unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> i32;
type LoadFn = unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> *mut c_void;
type EvalFn = unsafe extern "C" fn(
    *const c_void,
    *const f64,
    usize,
    *const f64,
    usize,
    *mut f64,
    *mut *mut c_char,
) -> i32;
type GuessFn =
    unsafe extern "C" fn(*const c_void, *const f64, *const f64, usize, *mut f64, usize) -> i32;
type ReleaseFn = unsafe extern "C" fn(*mut c_void);
type FreeFn = unsafe extern "C" fn(*mut c_char);

struct Bridge {
    load: LoadFn,
    eval: EvalFn,
    guess: GuessFn,
    release: ReleaseFn,
    free: FreeFn,
    summary: String,
    _lib: Library,
}

static BRIDGE: OnceLock<Result<Bridge, String>> = OnceLock::new();

const PROBE: &str = "import sys, json, sysconfig\n\
try:\n    import numpy; np = numpy.__version__\n\
except Exception:\n    np = None\n\
print(json.dumps({'base_prefix': sys.base_prefix, 'version': list(sys.version_info[:2]), 'path': sys.path, 'numpy': np, \
'executable': sys.executable, 'libdir': sysconfig.get_config_var('LIBDIR'), 'soname': sysconfig.get_config_var('INSTSONAME')}))";

#[derive(Deserialize)]
struct Probe {
    #[cfg_attr(not(windows), allow(dead_code))]
    base_prefix: String,
    version: (u32, u32),
    path: Vec<String>,
    numpy: Option<String>,
    executable: String,
    #[cfg_attr(not(unix), allow(dead_code))]
    libdir: Option<String>,
    #[cfg_attr(not(unix), allow(dead_code))]
    soname: Option<String>,
}

/// Human-readable interpreter state for the Plugins window.
pub fn status() -> String {
    match BRIDGE.get() {
        None => "not started (no Python models loaded yet)".into(),
        Some(Ok(b)) => b.summary.clone(),
        Some(Err(e)) => format!("unavailable: {e}"),
    }
}

fn probe(python: &str) -> Result<Probe, String> {
    let mut cmd = std::process::Command::new(python);
    cmd.args(["-c", PROBE]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| format!("could not run `{python}`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`{python}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).map_err(|e| format!("unexpected output from `{python}`: {e}"))
}

/// Load the interpreter's shared library by full path so the bridge's import of
/// it binds to this interpreter rather than whatever the loader would find.
fn preload_python(p: &Probe) -> Result<(), String> {
    #[cfg(windows)]
    {
        let home = Path::new(&p.base_prefix);
        let versioned = home.join(format!("python{}{}.dll", p.version.0, p.version.1));
        for dll in [versioned, home.join("python3.dll")] {
            let lib =
                unsafe { Library::new(&dll) }.map_err(|e| format!("{}: {e}", dll.display()))?;
            std::mem::forget(lib);
        }
    }
    #[cfg(unix)]
    {
        use libloading::os::unix::{Library as UnixLib, RTLD_GLOBAL, RTLD_NOW};
        if let (Some(dir), Some(name)) = (&p.libdir, &p.soname) {
            // Best effort: statically linked Pythons have no shared library.
            if let Ok(lib) =
                unsafe { UnixLib::open(Some(Path::new(dir).join(name)), RTLD_NOW | RTLD_GLOBAL) }
            {
                std::mem::forget(lib);
            }
        }
    }
    Ok(())
}

fn bridge_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "fit_it_py.dll"
    } else if cfg!(target_os = "macos") {
        "libfit_it_py.dylib"
    } else {
        "libfit_it_py.so"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    // Next to the exe; one up for test binaries in target/<profile>/deps; lib/ for Linux packages.
    [
        dir.join(name),
        dir.join("..").join(name),
        dir.join("../lib/fit_it").join(name),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// # Safety
/// `s` is null or a string allocated by the bridge.
unsafe fn take_string(free: FreeFn, s: *mut c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    unsafe { free(s) };
    out
}

fn start(python: &str) -> Result<Bridge, String> {
    let p = probe(python)?;
    if p.version < (3, 9) {
        return Err(format!(
            "Python {}.{} is too old (need 3.9+)",
            p.version.0, p.version.1
        ));
    }
    if p.numpy.is_none() {
        return Err(format!("numpy is not installed for {}", p.executable));
    }
    preload_python(&p)?;
    let path =
        bridge_path().ok_or("the Python bridge library (fit_it_py) is missing next to the app")?;
    let lib = unsafe { Library::new(&path) }.map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: symbol types match crates/fit_it_py/src/lib.rs, checked by the ABI number.
    unsafe {
        let abi = lib
            .get::<unsafe extern "C" fn() -> u32>(b"fit_it_py_abi\0")
            .map_err(|e| e.to_string())?;
        if abi() != BRIDGE_ABI {
            return Err("the Python bridge library does not match this app version".into());
        }
        let init = *lib
            .get::<InitFn>(b"fit_it_py_init\0")
            .map_err(|e| e.to_string())?;
        let load = *lib
            .get::<LoadFn>(b"fit_it_py_load\0")
            .map_err(|e| e.to_string())?;
        let eval = *lib
            .get::<EvalFn>(b"fit_it_py_eval\0")
            .map_err(|e| e.to_string())?;
        let guess = *lib
            .get::<GuessFn>(b"fit_it_py_guess\0")
            .map_err(|e| e.to_string())?;
        let release = *lib
            .get::<ReleaseFn>(b"fit_it_py_release\0")
            .map_err(|e| e.to_string())?;
        let free = *lib
            .get::<FreeFn>(b"fit_it_py_free_string\0")
            .map_err(|e| e.to_string())?;

        let sys_path =
            CString::new(serde_json::to_string(&p.path).unwrap_or_default()).unwrap_or_default();
        let mut info = std::ptr::null_mut();
        let rc = init(sys_path.as_ptr(), &mut info);
        let info = take_string(free, info);
        if rc != 0 {
            return Err(format!("starting Python failed: {info}"));
        }
        Ok(Bridge {
            load,
            eval,
            guess,
            release,
            free,
            summary: format!(
                "Python {}.{} ({}), numpy {info}",
                p.version.0, p.version.1, p.executable
            ),
            _lib: lib,
        })
    }
}

fn bridge(python: &str) -> Result<&'static Bridge, String> {
    // An interpreter can only be embedded once per process; a changed Python
    // setting takes effect after restarting the app.
    BRIDGE
        .get_or_init(|| start(python))
        .as_ref()
        .map_err(Clone::clone)
}

#[derive(Deserialize)]
struct ParamMeta {
    name: String,
    unit: String,
    description: String,
    default: f64,
    min: Option<f64>,
    max: Option<f64>,
    #[serde(default = "yes")]
    vary: bool,
    #[serde(default)]
    expr: String,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    category: String,
    description: String,
    params: Vec<ParamMeta>,
}

pub struct PyModel {
    info: ModelInfo,
    handle: *mut c_void,
    bridge: &'static Bridge,
}

// SAFETY: the handle is only used through the bridge, which holds the GIL while
// touching Python objects.
unsafe impl Send for PyModel {}
unsafe impl Sync for PyModel {}

pub fn load(path: &Path, python: &str) -> Result<Vec<Box<dyn Model>>, String> {
    let b = bridge(python)?;
    let cpath = CString::new(path.to_string_lossy().as_ref()).map_err(|e| e.to_string())?;
    let mut meta = std::ptr::null_mut();
    let handle = unsafe { (b.load)(cpath.as_ptr(), &mut meta) };
    let meta = unsafe { take_string(b.free, meta) };
    if handle.is_null() {
        return Err(meta);
    }
    let m: Meta = match serde_json::from_str(&meta) {
        Ok(m) => m,
        Err(e) => {
            unsafe { (b.release)(handle) };
            return Err(format!("bad model description: {e}"));
        }
    };
    let info = ModelInfo {
        name: m.name,
        category: m.category,
        description: m.description,
        params: m
            .params
            .into_iter()
            .map(|p| ParamDef {
                name: p.name,
                unit: p.unit,
                description: p.description,
                default: p.default,
                min: p.min.unwrap_or(f64::NEG_INFINITY),
                max: p.max.unwrap_or(f64::INFINITY),
                vary: p.vary,
                expr: p.expr,
            })
            .collect(),
    };
    Ok(vec![Box::new(PyModel {
        info,
        handle,
        bridge: b,
    })])
}

impl Model for PyModel {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn eval(&self, x: &[f64], p: &[f64], out: &mut [f64]) -> Result<(), String> {
        let mut err = std::ptr::null_mut();
        let rc = unsafe {
            (self.bridge.eval)(
                self.handle,
                x.as_ptr(),
                x.len(),
                p.as_ptr(),
                p.len(),
                out.as_mut_ptr(),
                &mut err,
            )
        };
        let err = unsafe { take_string(self.bridge.free, err) };
        if rc == 0 { Ok(()) } else { Err(err) }
    }

    fn guess(&self, x: &[f64], y: &[f64]) -> Option<Vec<f64>> {
        let n = x.len().min(y.len());
        let mut p: Vec<f64> = self.info.params.iter().map(|p| p.default).collect();
        let rc = unsafe {
            (self.bridge.guess)(
                self.handle,
                x.as_ptr(),
                y.as_ptr(),
                n,
                p.as_mut_ptr(),
                p.len(),
            )
        };
        (rc == 0).then_some(p)
    }
}

impl Drop for PyModel {
    fn drop(&mut self) {
        unsafe { (self.bridge.release)(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs only where a Python with numpy is on PATH and the bridge was built.
    #[test]
    fn sasview_style_model_through_the_bridge() {
        if probe("python").map(|p| p.numpy.is_none()).unwrap_or(true) || bridge_path().is_none() {
            eprintln!("skipped: needs python + numpy and the fit_it_py bridge");
            return;
        }
        let file = crate::plugin::find_presets_dir()
            .unwrap()
            .join("lorentz.py");
        let models = load(&file, "python").unwrap();
        let m = &models[0];
        let names: Vec<_> = m.info().params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["scale", "background", "cor_length"]);
        let mut out = [0.0; 2];
        m.eval(&[0.0, 0.02], &[2.0, 0.5, 50.0], &mut out).unwrap();
        assert!((out[0] - 2.5).abs() < 1e-12);
        assert!((out[1] - (2.0 / 2.0 + 0.5)).abs() < 1e-12);
        let err = m.eval(&[0.0], &[1.0], &mut [0.0]).unwrap_err();
        assert!(err.contains("mismatch"), "{err}");
    }
}
