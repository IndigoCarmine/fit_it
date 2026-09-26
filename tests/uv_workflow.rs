//! End-to-end check of the UV-Vis supramolecular-polymerization workflow on real
//! JASCO data: load temperature scans, take the cross-section at one wavelength,
//! and globally fit TempCooperative with shared ΔH, ΔS, ΔHnuc and one scaler per
//! concentration.
//!
//! Needs measurement data, so it only runs when `FIT_IT_UV_DIR` points at a folder
//! with 2-D JASCO `.txt` files whose names contain the concentration
//! (e.g. `..._50microM_...`):
//!
//!     FIT_IT_UV_DIR=UV/Quin FIT_IT_UV_AT=295 FIT_IT_UV_INVERT=1 cargo test --test uv_workflow -- --nocapture

use fit_it::data::Dataset;
use fit_it::fit::fit;
use fit_it::model::composite::Component;
use fit_it::plugin::{Folder, LoadOptions, Registry, find_presets_dir};
use fit_it::project::{DerivedSpec, Project};
use fit_it::transform::{SliceOptions, slice_columns};

#[test]
fn global_temp_cooperative_fit_on_jasco_scans() {
    let Ok(dir) = std::env::var("FIT_IT_UV_DIR") else {
        eprintln!("skipped: set FIT_IT_UV_DIR to run");
        return;
    };
    let at: f64 = std::env::var("FIT_IT_UV_AT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300.0);
    let invert = std::env::var("FIT_IT_UV_INVERT").is_ok_and(|v| v == "1");

    let registry = Registry::load(
        &[Folder {
            label: "Presets".into(),
            path: find_presets_dir().expect("presets next to the test binary"),
        }],
        &LoadOptions {
            python: "python".into(),
        },
    );
    let lookup = |n: &str| registry.get(n);

    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    files.sort();

    let opts = SliceOptions {
        at,
        baseline: Some((400.0, f64::INFINITY)),
        x_offset: 273.15,
        normalize: true,
        invert,
        x_label: "T (K)".into(),
        ..Default::default()
    };
    let mut project = Project::default();
    for f in &files {
        let raw = Dataset::load(f).unwrap();
        let curve = slice_columns(&raw, &opts).unwrap();
        println!(
            "{}: {} points, conc_M = {:e}",
            raw.name,
            curve.table.rows(),
            curve.constants["conc_M"]
        );
        let i = project.add_dataset(curve);
        project.datasets[i].spec.components.push(Component {
            name: "t1".into(),
            model: "TempCooperative".into(),
        });
    }
    project.sync_params(&lookup);
    for d in &mut project.datasets {
        // c_tot follows the concentration in the file name (model default).
        assert_eq!(d.param("t1_c_tot").unwrap().expr, "conc_M");
        d.data.range = Some((20.0 + 273.15, f64::INFINITY));
        for p in &mut d.params {
            match p.name.as_str() {
                // Starting point and bounds as in fitting.py (ΔHnuc sign flipped:
                // sp_fitting_models uses sigma = exp(-ΔHnuc/RT)).
                "t1_deltaH" => (p.value, p.min, p.max) = (-89841.0, -158000.0, 0.0),
                "t1_deltaS" => (p.value, p.min, p.max) = (-162.0, -2000.0, -10.0),
                "t1_deltaHnuc" => (p.value, p.min, p.max) = (5519.0, 1000.0, 40000.0),
                "t1_scaler" => (p.value, p.min, p.max) = (1.0, 0.0, 2.0),
                _ => {}
            }
        }
    }
    for name in ["t1_deltaH", "t1_deltaS", "t1_deltaHnuc"] {
        project.share_param(0, name);
    }
    project.derived = vec![
        DerivedSpec {
            name: "deltaG_300K".into(),
            expr: "t1_deltaH - 300 * t1_deltaS".into(),
        },
        DerivedSpec {
            name: "K_elong_300K".into(),
            expr: "exp(-(t1_deltaH - 300 * t1_deltaS) / (8.314 * 300))".into(),
        },
    ];

    let problem = project.problem(&lookup, &|_| true).unwrap();
    let mut out = fit(&problem, &project.options, None, None).unwrap();
    out.derived = fit_it::fit::derive(
        &problem,
        &out,
        &DerivedSpec::pairs(&project.derived),
        Some("D1"),
    );
    println!("{}", out.report());
    assert!(out.success, "{}", out.message);

    // Optional: write the exports for inspection (FIT_IT_REPORT_DIR=some/dir).
    if let Ok(dir) = std::env::var("FIT_IT_REPORT_DIR") {
        project.apply_outcome(&out);
        let set = fit_it::export::export_set(&project, 0);
        let curves = fit_it::export::dataset_curves(&project, &lookup, &set, 600, false).unwrap();
        let dir = std::path::Path::new(&dir);
        std::fs::write(
            dir.join("global_fit.csv"),
            fit_it::export::side_by_side_csv(&curves),
        )
        .unwrap();
        let report = format!("Global fit\n{}", out.report());
        let pdf = fit_it::report_pdf::render(&fit_it::report_pdf::ReportInput {
            title: "fit_it report".into(),
            info: vec![
                "Global fit (TempCooperative, shared ΔH / ΔS / ΔHnuc)".into(),
                "グローバルフィット: 濃度 50 / 10 / 5 µM、χ² 最小化".into(),
            ],
            curves: &curves,
            colors: set.clone(),
            params: set
                .iter()
                .map(|&i| project.datasets[i].params.clone())
                .collect(),
            report_text: &report,
            log_x: false,
            log_y: false,
        });
        std::fs::write(dir.join("fit_report.pdf"), pdf).unwrap();
    }
    for (tag, r2) in &out.r2 {
        assert!(*r2 > 0.75, "{tag}: R² = {r2}");
    }
}

/// titration.py: absorbance at 400 nm / concentration vs concentration, one
/// spectrum per file, fitted with the isothermal cooperative model.
///
///     FIT_IT_UV_TITRATION_DIR=UV/THF_denaturation cargo test --test uv_workflow -- --nocapture
#[test]
fn cooperative_fit_on_titration_series() {
    let Ok(dir) = std::env::var("FIT_IT_UV_TITRATION_DIR") else {
        eprintln!("skipped: set FIT_IT_UV_TITRATION_DIR to run");
        return;
    };
    let registry = Registry::load(
        &[Folder {
            label: "Presets".into(),
            path: find_presets_dir().unwrap(),
        }],
        &LoadOptions {
            python: "python".into(),
        },
    );
    let lookup = |n: &str| registry.get(n);
    let mut spectra: Vec<Dataset> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .map(|p| Dataset::load(&p).unwrap())
        .collect();
    spectra.sort_by(|a, b| a.name.cmp(&b.name));
    let refs: Vec<&Dataset> = spectra.iter().collect();
    let opts = SliceOptions {
        at: 400.0,
        baseline: Some((450.0, f64::INFINITY)),
        divide_by: Some("conc_uM".into()),
        x_scale: 1e-6,
        normalize: true,
        x_label: "c (M)".into(),
        ..Default::default()
    };
    let curve = fit_it::transform::slice_across(&refs, "conc_uM", &opts).unwrap();
    println!(
        "x = {:?}
y = {:?}",
        curve.x(),
        curve.y()
    );

    let mut project = Project::default();
    let i = project.add_dataset(curve);
    project.datasets[i].spec.components.push(Component {
        name: "c1".into(),
        model: "Cooperative".into(),
    });
    project.sync_params(&lookup);
    // Starting values from titration.py.
    for (name, v) in [
        ("c1_K", 16759.5088),
        ("c1_sigma", 0.00339429),
        ("c1_scaler", 2.15507119),
    ] {
        project.datasets[i]
            .params
            .iter_mut()
            .find(|p| p.name == name)
            .unwrap()
            .value = v;
    }
    project.derived = vec![DerivedSpec {
        name: "Gibbs_kJ_per_mol".into(),
        expr: "-ln(c1_K) * 298 / 1000 * 1.9872036 * 4.184".into(),
    }];
    let problem = project.problem(&lookup, &|_| true).unwrap();
    let mut out = fit(&problem, &project.options, None, None).unwrap();
    out.derived = fit_it::fit::derive(
        &problem,
        &out,
        &DerivedSpec::pairs(&project.derived),
        Some("D1"),
    );
    println!("{}", out.report());
    assert!(out.success, "{}", out.message);
}
