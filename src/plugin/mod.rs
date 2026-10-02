//! Model folders → models.
//!
//! Presets and user plugins are handled identically: each is a folder scanned by
//! [`Registry::load`]. What a file becomes depends only on its extension:
//!
//! | file                    | loaded as                                     |
//! |-------------------------|-----------------------------------------------|
//! | `*.c`                   | compiled to a shared library (cached in `.build/`), then loaded |
//! | `*.dll` `*.so` `*.dylib`| prebuilt library implementing `fit_it_plugin.h` |
//! | `*.py`                  | SasView-style model in the embedded Python    |
//! | `*.fexpr`               | formula model (TOML)                          |
//!
//! Later folders win on name clashes, so a plugin can replace a preset.

pub mod compile;
pub mod native;
pub mod python;

use crate::model::ModelRef;
use crate::model::expression::ExprModel;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Target triple the app was built for; C plugins are compiled for it.
pub const TARGET: &str = env!("FIT_IT_TARGET");

pub const HEADER: &str = include_str!("../../resources/plugin_templates/fit_it_plugin.h");
const TEMPLATE_C: &str = include_str!("../../resources/plugin_templates/template.c");
const TEMPLATE_PY: &str = include_str!("../../resources/plugin_templates/template.py");
const TEMPLATE_FEXPR: &str = include_str!("../../resources/plugin_templates/template.fexpr");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    C,
    Library,
    Python,
    Formula,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::C => "C",
            SourceKind::Library => "library",
            SourceKind::Python => "Python",
            SourceKind::Formula => "formula",
        }
    }

    fn of(path: &Path) -> Option<SourceKind> {
        Self::from_extension(path.extension()?.to_str()?)
    }

    /// What a file with extension `ext` (without the dot, case-sensitive) loads as.
    pub fn from_extension(ext: &str) -> Option<SourceKind> {
        match ext {
            "c" => Some(SourceKind::C),
            "dll" | "so" | "dylib" => Some(SourceKind::Library),
            "py" => Some(SourceKind::Python),
            "fexpr" => Some(SourceKind::Formula),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Entry {
    pub model: ModelRef,
    pub folder: String,
    pub file: PathBuf,
    pub kind: SourceKind,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub file: PathBuf,
    pub message: String,
    /// Warnings (e.g. a plugin overriding a preset) do not stop anything from loading.
    pub warning: bool,
}

#[derive(Clone, Default)]
pub struct Registry {
    pub entries: Vec<Entry>,
    pub issues: Vec<Issue>,
    pub python_status: String,
}

#[derive(Clone, Debug)]
pub struct LoadOptions {
    /// Python interpreter used for `.py` models.
    pub python: String,
}

impl Registry {
    pub fn load(folders: &[Folder], opts: &LoadOptions) -> Self {
        let mut reg = Registry::default();
        for folder in folders {
            reg.scan(folder, opts);
        }
        reg.python_status = python::status();
        reg
    }

    fn issue(&mut self, file: &Path, message: impl Into<String>, warning: bool) {
        self.issues.push(Issue {
            file: file.to_path_buf(),
            message: message.into(),
            warning,
        });
    }

    fn scan(&mut self, folder: &Folder, opts: &LoadOptions) {
        let Ok(dir) = std::fs::read_dir(&folder.path) else {
            self.issue(&folder.path, "folder not found", true);
            return;
        };
        let mut files: Vec<PathBuf> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for file in files {
            let Some(kind) = SourceKind::of(&file) else {
                continue;
            };
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.starts_with('_') || stem.starts_with('.') {
                continue; // helpers imported by other files
            }
            let loaded: Result<Vec<ModelRef>, String> = match kind {
                SourceKind::C => compile::build_cached(&file, &folder.path.join(".build"), TARGET)
                    .and_then(|(lib, _)| native::load(&lib))
                    .map(|ms| ms.into_iter().map(|m| Arc::new(m) as ModelRef).collect()),
                SourceKind::Library => native::load(&file)
                    .map(|ms| ms.into_iter().map(|m| Arc::new(m) as ModelRef).collect()),
                SourceKind::Python => python::load(&file, &opts.python)
                    .map(|ms| ms.into_iter().map(ModelRef::from).collect()),
                SourceKind::Formula => {
                    ExprModel::load_file(&file).map(|m| vec![Arc::new(m) as ModelRef])
                }
            };
            match loaded {
                Ok(models) => {
                    for model in models {
                        self.add(Entry {
                            model,
                            folder: folder.label.clone(),
                            file: file.clone(),
                            kind,
                        });
                    }
                }
                Err(e) => self.issue(&file, e, false),
            }
        }
    }

    fn add(&mut self, entry: Entry) {
        let name = entry.model.info().name.clone();
        if let Some(bad) = entry
            .model
            .info()
            .params
            .iter()
            .find(|p| !crate::model::is_identifier(&p.name))
        {
            let msg = format!("{name}: parameter name `{}` is not an identifier", bad.name);
            self.issue(&entry.file, msg, false);
            return;
        }
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.model.info().name == name)
        {
            let old = &self.entries[i];
            let msg = format!(
                "`{name}` replaces the one from {} ({})",
                old.folder,
                old.file.display()
            );
            self.issue(&entry.file, msg, true);
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn get(&self, name: &str) -> Option<ModelRef> {
        self.find(name).map(|e| e.model.clone())
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.model.info().name == name)
    }

    /// Entries grouped by category, categories and names sorted.
    pub fn by_category(&self) -> Vec<(String, Vec<&Entry>)> {
        let mut groups: std::collections::BTreeMap<String, Vec<&Entry>> = Default::default();
        for e in &self.entries {
            let cat = e.model.info().category.clone();
            groups
                .entry(if cat.is_empty() { "Other".into() } else { cat })
                .or_default()
                .push(e);
        }
        groups
            .into_iter()
            .map(|(k, mut v)| {
                v.sort_by(|a, b| a.model.info().name.cmp(&b.model.info().name));
                (k, v)
            })
            .collect()
    }
}

/// The presets folder shipped with the app, wherever the platform put it.
pub fn find_presets_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("presets"),
        dir.join("../presets"), // test binaries in target/<profile>/deps
        dir.join("../share/fit_it/presets"), // Linux packages
        dir.join("../Resources/presets"), // macOS bundle
    ]
    .into_iter()
    .find(|p| p.is_dir())
    .map(|p| {
        let p = p.canonicalize().unwrap_or(p);
        // Windows canonical paths carry a `\\?\` prefix that users should not see.
        match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(s) => PathBuf::from(s),
            None => p,
        }
    })
}

pub fn default_plugin_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fit_it")
        .join("plugins")
}

/// Create the plugin folder and keep its copy of the C header current.
pub fn prepare_plugin_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let header = dir.join("fit_it_plugin.h");
    if std::fs::read_to_string(&header).ok().as_deref() != Some(HEADER) {
        std::fs::write(&header, HEADER).map_err(|e| format!("{}: {e}", header.display()))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    C,
    Python,
    Formula,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::Python, Template::C, Template::Formula];

    pub fn label(self) -> &'static str {
        match self {
            Template::C => "C model (.c)",
            Template::Python => "Python model (.py, SasView style)",
            Template::Formula => "Formula model (.fexpr)",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Template::C => "c",
            Template::Python => "py",
            Template::Formula => "fexpr",
        }
    }
}

/// Write a new model file from a template; `name` becomes the model name.
pub fn write_template(dir: &Path, template: Template, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if !crate::model::is_identifier(name) {
        return Err("use letters, digits and '_' for the model name".into());
    }
    prepare_plugin_dir(dir)?;
    let path = dir.join(format!("{}.{}", name.to_lowercase(), template.ext()));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let text = match template {
        Template::C => TEMPLATE_C,
        Template::Python => TEMPLATE_PY,
        Template::Formula => TEMPLATE_FEXPR,
    }
    .replace("MyModel", name);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Open a file or folder with the desktop's default handler.
pub fn open_in_os(path: &Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presets() -> Registry {
        let dir = find_presets_dir().expect("build.rs puts presets next to the test binary");
        Registry::load(
            &[Folder {
                label: "Presets".into(),
                path: dir,
            }],
            &LoadOptions {
                python: "python".into(),
            },
        )
    }

    #[test]
    fn presets_load_through_the_plugin_path() {
        let reg = presets();
        let errors: Vec<_> = reg
            .issues
            .iter()
            .filter(|i| !i.warning && !i.file.ends_with("lorentz.py"))
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        for name in [
            "Gaussian",
            "Voigt",
            "Linear",
            "Sphere",
            "Sine",
            "StretchedExponential",
        ] {
            assert!(reg.get(name).is_some(), "{name} missing");
        }
    }

    #[test]
    fn c_preset_values_and_guess() {
        let reg = presets();
        let g = reg.get("Gaussian").unwrap();
        let mut out = [0.0; 1];
        g.eval(&[1.0], &[2.0, 1.0, 0.5], &mut out).unwrap();
        assert!((out[0] - 2.0 / (0.5 * (2.0 * std::f64::consts::PI).sqrt())).abs() < 1e-12);

        // Voigt with tiny gamma approaches the Gaussian.
        let v = reg.get("Voigt").unwrap();
        let mut vo = [0.0; 1];
        v.eval(&[1.0], &[2.0, 1.0, 0.5, 1e-6], &mut vo).unwrap();
        assert!(
            (vo[0] - out[0]).abs() / out[0] < 1e-3,
            "{} vs {}",
            vo[0],
            out[0]
        );

        let x: Vec<f64> = (0..100).map(|i| i as f64 * 0.1).collect();
        let mut y = vec![0.0; x.len()];
        g.eval(&x, &[3.0, 4.0, 0.6], &mut y).unwrap();
        let guess = g.guess(&x, &y).unwrap();
        assert!((guess[1] - 4.0).abs() < 0.11);
        assert!((guess[2] - 0.6).abs() < 0.1);
    }

    /// Reference values computed with sp_fitting_models 1.3.9 (the library the
    /// supramolecular preset is ported from).
    #[test]
    fn supramolecular_preset_matches_sp_fitting_models() {
        let reg = presets();
        let t = [300.0, 330.0, 350.0];
        // (model, x, parameters, expected)
        type Case<'a> = (&'a str, &'a [f64], &'a [f64], [f64; 3]);
        let conc = [1e-6, 1e-5, 1e-4];
        let cases: [Case; 8] = [
            (
                "TempCooperative",
                &t,
                &[-100000.0, -200.0, 10000.0, 2e-5, 1.0],
                [0.994632390466696, 0.8087563677092208, 0.10551121699880173],
            ),
            (
                "TempIsodesmic",
                &t,
                &[-80000.0, -150.0, 2e-5, 1.0],
                [0.9670595255408223, 0.6783452815195611, 0.31713720353909947],
            ),
            (
                "TempCoopIso",
                &t,
                &[-70000.0, -150.0, -100000.0, -200.0, 10000.0, 2e-5, 1.0],
                [0.9946323911663294, 0.8087810838595844, 0.11441640712630385],
            ),
            (
                "Cooperative",
                &[1e-6, 1e-5, 1e-4],
                &[1e5, 0.01, 1.0],
                [
                    0.0023338192519988255,
                    0.19756346807866443,
                    0.903270677349801,
                ],
            ),
            (
                "TempCooperativeN",
                &t,
                &[-100000.0, -200.0, 10000.0, 4.0, 2e-5, 1.0],
                [0.9945797238901102, 0.792947743805153, 0.03691620570500309],
            ),
            (
                "CooperativeN",
                &conc,
                &[1e5, 0.01, 3.0, 1.0],
                [0.001995461222902506, 0.0523147842294962, 0.9003330783466562],
            ),
            (
                "CoopIso",
                &conc,
                &[1e4, 1e5, 0.01, 1.0],
                [0.021664365948949826, 0.2456227659903475, 0.9033100962079069],
            ),
            (
                "Isodesmic",
                &conc,
                &[1e5, 1.0],
                [0.1607978309961604, 0.6180339887498949, 0.9270156211871643],
            ),
        ];
        for (name, x, p, want) in cases {
            let m = reg.get(name).unwrap_or_else(|| panic!("{name} missing"));
            let mut out = [0.0; 3];
            m.eval(x, p, &mut out).unwrap();
            for (o, w) in out.iter().zip(want) {
                assert!((o - w).abs() < 1e-9, "{name}: {o} vs {w}");
            }
        }
        // Extreme parameters stay finite and in [0, 1]; a tiny K means no aggregation.
        let coop = reg.get("Cooperative").unwrap();
        let mut out = [0.0; 3];
        coop.eval(&conc, &[1e-12, 0.01, 1.0], &mut out).unwrap();
        assert!(out.iter().all(|&o| (0.0..1e-12).contains(&o)), "{out:?}");
        coop.eval(&conc, &[1e300, 1.0, 1.0], &mut out).unwrap();
        assert!(out.iter().all(|&o| (o - 1.0).abs() < 1e-9), "{out:?}");
        // As upstream: a negative K is NaN, a non-positive concentration or temperature an error.
        coop.eval(&conc, &[-1.0, 0.01, 1.0], &mut out).unwrap();
        assert!(out.iter().all(|o| o.is_nan()), "{out:?}");
        assert!(
            coop.eval(&[0.0, 1e-5, 1e-4], &[1e5, 0.01, 1.0], &mut out)
                .is_err()
        );
        let tc = reg.get("TempCooperative").unwrap();
        assert!(
            tc.eval(
                &[0.0, 300.0, 330.0],
                &[-1e5, -200.0, 1e4, 2e-5, 1.0],
                &mut out
            )
            .is_err()
        );
        assert!(
            tc.eval(&t, &[-1e5, -200.0, 1e4, 0.0, 1.0], &mut out)
                .is_err()
        );

        // c_tot is a known input: fixed, and linked to the file's concentration.
        let model = reg.get("TempCooperative").unwrap();
        let c_tot = &model.info().params[3];
        assert_eq!(
            (c_tot.name.as_str(), c_tot.vary, c_tot.expr.as_str()),
            ("c_tot", false, "conc_M")
        );
    }

    #[test]
    fn templates_load_as_plugins() {
        let dir = std::env::temp_dir().join(format!("fit_it_tpl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_template(&dir, Template::Formula, "MyFormula").unwrap();
        write_template(&dir, Template::C, "MyC").unwrap();
        assert!(write_template(&dir, Template::C, "MyC").is_err());
        assert!(write_template(&dir, Template::C, "bad name").is_err());
        let reg = Registry::load(
            &[Folder {
                label: "Plugins".into(),
                path: dir.clone(),
            }],
            &LoadOptions {
                python: "python".into(),
            },
        );
        assert!(reg.get("MyFormula").is_some());
        let c_ok = reg.get("MyC").is_some();
        let no_cc = reg
            .issues
            .iter()
            .any(|i| i.message.contains("no C compiler"));
        assert!(c_ok || no_cc, "{:?}", reg.issues);
        if c_ok {
            let mut out = [0.0];
            reg.get("MyC")
                .unwrap()
                .eval(&[0.0], &[2.0, 1.0, 0.5], &mut out)
                .unwrap();
            assert_eq!(out[0], 2.5);
        } else {
            eprintln!("C template not compiled: no C compiler on this machine");
        }
        drop(reg);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
