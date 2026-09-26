//! Generates the synthetic data and the screenshots of the global-fit manual
//! (docs/manual). Drives the real app headlessly with egui_kittest and renders
//! each step with wgpu, so the manual can be rebuilt whenever the UI changes:
//!
//!     cargo test --test manual_screenshots -- --ignored --nocapture
//!     typst compile docs/manual/global_fit.typ

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use fit_it::FitApp;
use fit_it::plugin::{Folder, LoadOptions, Registry, find_presets_dir};
use std::path::{Path, PathBuf};

/// Parameters the fake data are generated from (what the fit should recover).
const DELTA_H: f64 = -90_000.0; // J/mol
const DELTA_S: f64 = -180.0; // J/(mol K)
const DELTA_H_NUC: f64 = 10_000.0; // J/mol
/// Concentration (µM) and signal scaler of each fake measurement.
const SAMPLES: [(f64, f64); 4] = [(5.0, 0.98), (10.0, 1.02), (20.0, 0.97), (50.0, 1.01)];
const NOISE: f64 = 0.012;

/// Relative to the package root (the test's working directory), so the status
/// bar in the screenshots shows a short path.
fn manual_dir() -> PathBuf {
    PathBuf::from("docs/manual")
}

/// Small deterministic Gaussian noise source (xorshift + Box–Muller), so the
/// data files do not change between runs.
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-300), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// Writes one cooling curve per concentration, evaluated with the shipped
/// TempCooperative preset plus noise. The concentration is in the file name so
/// the app reads it as the constant `conc_M`.
fn write_fake_data(dir: &Path) -> Vec<PathBuf> {
    let registry = Registry::load(
        &[Folder {
            label: "Presets".into(),
            path: find_presets_dir().expect("presets next to the test binary"),
        }],
        &LoadOptions {
            python: "python".into(),
        },
    );
    let model = registry
        .get("TempCooperative")
        .expect("TempCooperative preset");
    std::fs::create_dir_all(dir).unwrap();
    let t: Vec<f64> = (0..=180).map(|i| 283.15 + 0.5 * i as f64).collect();
    let mut noise = Noise(0x5eed_1234_abcd_0001);
    let mut files = Vec::new();
    for (conc_um, scaler) in SAMPLES {
        let p = [DELTA_H, DELTA_S, DELTA_H_NUC, conc_um * 1e-6, scaler];
        let mut y = vec![0.0; t.len()];
        model.eval(&t, &p, &mut y).unwrap();
        let mut text =
            String::from("# Synthetic cooling curve (TempCooperative + noise)\nT_K\taggregated\n");
        for (x, y) in t.iter().zip(&y) {
            text.push_str(&format!("{x:.2}\t{:.5}\n", y + NOISE * noise.gauss()));
        }
        let path = dir.join(format!("sample_{conc_um}microM_cooling.txt"));
        std::fs::write(&path, text).unwrap();
        files.push(path);
    }
    files
}

struct Shots {
    dir: PathBuf,
    /// Name of the last screenshot, the source of [`Shots::crop`].
    last: std::cell::RefCell<String>,
}

impl Shots {
    fn save(&self, h: &mut Harness<'_, FitApp>, name: &str) {
        // Rendered at pixels_per_point 1: kittest's pointer positions are
        // only right at 1:1.
        h.remove_cursor();
        h.run_steps(4);
        let img = h.render().expect("render");
        img.save(self.dir.join(format!("{name}.png"))).unwrap();
        println!("saved {name}.png");
        *self.last.borrow_mut() = name.to_string();
    }
}

/// Clicks the parameter-table cell in the row labelled `row` and the column
/// headed `col`, selects its text and types `text` (+ Enter).
fn edit_cell(h: &mut Harness<'_, FitApp>, row: &str, col: &str, text: &str) {
    let y = h.get_by_label(row).rect().center().y;
    // The rightmost header of that name is the parameter table's.
    let x = h
        .get_all_by_label(col)
        .map(|n| n.rect().center().x)
        .fold(f32::MIN, f32::max);
    let pos = egui::pos2(x, y);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn select_dataset(h: &mut Harness<'_, FitApp>, tag: &str) {
    h.get_by_label_contains(&format!("{tag}  sample_")).click();
    h.run_steps(2);
}

/// Where a close-up of the last screenshot should go: the union of the named
/// widgets' rectangles (exact label, else the last node containing it), widened
/// by `pad`, optionally stretched to the left (`x0`) or right window edge.
struct Region<'a> {
    labels: &'a [&'a str],
    pad: f32,
    x0: Option<f32>,
    to_right_edge: bool,
}

impl Shots {
    fn crop(&self, h: &Harness<'_, FitApp>, name: &str, r: Region<'_>) {
        let mut rect = egui::Rect::NOTHING;
        for l in r.labels {
            let node = h
                .query_by_label(l)
                .or_else(|| h.query_all_by_label_contains(l).last())
                .unwrap_or_else(|| panic!("no node `{l}` for {name}"));
            rect = rect.union(node.rect());
        }
        let mut rect = rect.expand(r.pad);
        if let Some(x0) = r.x0 {
            rect.min.x = x0;
        }
        if r.to_right_edge {
            rect.max.x = 1360.0;
        }
        let rect = rect.intersect(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1360.0, 820.0),
        ));
        let img = image::open(self.dir.join(format!("{}.png", self.last.borrow()))).unwrap();
        img.crop_imm(
            rect.min.x as u32,
            rect.min.y as u32,
            rect.width() as u32,
            rect.height() as u32,
        )
        .save(self.dir.join(format!("{name}.png")))
        .unwrap();
    }
}

/// The parameter table from its heading to the `t1_scaler` row.
const PARAMS: Region<'static> = Region {
    labels: &["パラメータ", "t1_scaler"],
    pad: 6.0,
    x0: None,
    to_right_edge: true,
};

/// Steps the app until `label` shows up (background model loading, fitting).
fn wait_for(h: &mut Harness<'_, FitApp>, label: &str) {
    for _ in 0..600 {
        h.step();
        if h.query_all_by_label_contains(label).next().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("`{label}` never appeared");
}

#[test]
#[ignore = "writes docs/manual; run explicitly"]
fn manual_screenshots() {
    // SAFETY: set before any other thread starts; the app reads it on start-up
    // to pick the Japanese UI.
    unsafe { std::env::set_var("LANG", "ja_JP.UTF-8") };
    let dir = manual_dir();
    let files = write_fake_data(&dir.join("data"));
    let shots = Shots {
        dir: dir.join("images"),
        last: Default::default(),
    };
    std::fs::create_dir_all(&shots.dir).unwrap();

    let mut h = Harness::builder()
        .with_size([1360.0, 820.0])
        .wgpu()
        .build_eframe(|cc| FitApp::new(cc, &[]));
    wait_for(&mut h, "個のモデルを読み込みました");
    h.get_by_label_contains("Light").click();
    shots.save(&mut h, "01_start");

    for f in &files {
        h.input_mut().dropped_files.push(egui::DroppedFile {
            path: Some(f.clone()),
            ..Default::default()
        });
    }
    h.run_steps(4);
    shots.save(&mut h, "02_loaded");
    shots.crop(
        &h,
        "c02_datasets",
        Region {
            labels: &["データセット", "全データセットへ", "追加"],
            pad: 6.0,
            x0: Some(0.0),
            to_right_edge: false,
        },
    );

    // Model: TempCooperative on D1.
    h.get_by_label_contains("D1  sample_5microM").click();
    h.run_steps(2);
    h.get_by_label("➕ 成分を追加").click();
    h.run_steps(2);
    h.get_by_label_contains("Supramolecular").hover();
    h.run_steps(2);
    h.get_by_label("TempCooperative").hover();
    shots.save(&mut h, "03_add_component");
    h.get_by_label("TempCooperative").click();
    shots.save(&mut h, "04_params");
    shots.crop(&h, "c04_params", PARAMS);

    // Starting values and physically sensible bounds.
    for (row, value, min, max) in [
        ("t1_deltaH", "-80000", "-300000", "0"),
        ("t1_deltaS", "-150", "-1000", "0"),
        ("t1_deltaHnuc", "5000", "0", "50000"),
        ("t1_scaler", "1", "0", "2"),
    ] {
        edit_cell(&mut h, row, "値", value);
        edit_cell(&mut h, row, "最小", min);
        edit_cell(&mut h, row, "最大", max);
    }
    shots.save(&mut h, "05_bounds");

    // Same model and settings for every dataset.
    h.get_by_label("全体にコピー").hover();
    shots.save(&mut h, "06_copy_to_all");
    shots.crop(
        &h,
        "c06_copy",
        Region {
            labels: &["➕ 成分を追加", "全体にコピー", "TempCooperative", "結合式"],
            pad: 4.0,
            x0: Some(0.0),
            to_right_edge: false,
        },
    );
    h.get_by_label("全体にコピー").click();
    h.run_steps(2);
    select_dataset(&mut h, "D3");
    shots.save(&mut h, "07_copied_d3");

    // Share ΔH, ΔS and ΔHnuc; the scaler stays per dataset.
    select_dataset(&mut h, "D1");
    for (i, name) in ["t1_deltaH", "t1_deltaS", "t1_deltaHnuc"]
        .iter()
        .enumerate()
    {
        // Shared rows turn their ⚙ into ↔, so the first ⚙ is the next row.
        h.get_all_by_label("⚙").next().unwrap().click();
        h.run_steps(2);
        let item = format!("{name} を全データセットで共有");
        h.get_by_label(&item).hover();
        if i == 0 {
            shots.save(&mut h, "08_share_menu");
            shots.crop(
                &h,
                "c08_share_menu",
                Region {
                    labels: &["パラメータ", "t1_scaler", "既定値に戻す"],
                    pad: 6.0,
                    x0: None,
                    to_right_edge: true,
                },
            );
        }
        h.get_by_label(&item).click();
        h.run_steps(2);
    }
    shots.save(&mut h, "09_shared_d1");
    shots.crop(&h, "c09_shared_d1", PARAMS);
    select_dataset(&mut h, "D2");
    shots.save(&mut h, "10_shared_d2");
    shots.crop(&h, "c10_shared_d2", PARAMS);
    select_dataset(&mut h, "D1");

    // Derived quantities at 300 K from the presets.
    h.get_by_label_contains("派生量 (").click();
    h.run_steps(2);
    for preset in ["deltaG_300K", "K_elong_300K", "sigma_300K"] {
        h.get_by_label("プリセット").click();
        h.run_steps(2);
        let item = format!("{preset} = ");
        h.get_by_label_contains(&item).hover();
        if preset == "sigma_300K" {
            shots.save(&mut h, "11_derived_presets");
            shots.crop(
                &h,
                "c11_derived_presets",
                Region {
                    labels: &["派生量 (", "sigma_300K = ", "K_c_300K = "],
                    pad: 6.0,
                    x0: None,
                    to_right_edge: true,
                },
            );
        }
        h.get_by_label_contains(&item).click();
        h.run_steps(2);
    }
    shots.save(&mut h, "12_derived");

    // Fit options: multi-start LM guards against local minima.
    h.get_by_label("オプション").click();
    h.run_steps(2);
    // The combo box exposes its selected text as a value, not a label.
    h.get_by_value("Levenberg–Marquardt（局所）").click();
    h.run_steps(2);
    h.get_by_label("マルチスタート LM").hover();
    shots.save(&mut h, "13_algorithm");
    shots.crop(
        &h,
        "c13_algorithm",
        Region {
            labels: &["オプション", "ベイスンホッピング", "ftol"],
            pad: 6.0,
            x0: None,
            to_right_edge: true,
        },
    );
    h.get_by_label("マルチスタート LM").click();
    h.run_steps(2);
    shots.save(&mut h, "14_options");
    shots.crop(
        &h,
        "c14_options",
        Region {
            labels: &["オプション", "xtol"],
            pad: 6.0,
            x0: None,
            to_right_edge: true,
        },
    );
    h.get_by_label("オプション").click();
    h.run_steps(2);

    // Global fit.
    h.get_by_label_contains("グローバルフィット (4)").click();
    wait_for(&mut h, "フィットレポート");
    shots.save(&mut h, "15_result_d1");
    shots.crop(&h, "c15_params", PARAMS);
    // The report text box, for quoting in the manual.
    let report = h
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .filter_map(|n| n.value())
        .find(|v| v.contains("[[Fit Statistics]]"))
        .expect("fit report");
    std::fs::write(dir.join("data/fit_report.txt"), report).unwrap();
    select_dataset(&mut h, "D4");
    shots.save(&mut h, "16_result_d4");

    // All curves in one plot.
    h.get_by_label("表示").click();
    h.run_steps(2);
    h.get_by_label("グローバルフィット対象を重ねて表示").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    shots.save(&mut h, "17_overlay");

    // Saving and exporting.
    h.get_by_label("ファイル").click();
    h.run_steps(2);
    shots.save(&mut h, "18_file_menu");
    shots.crop(
        &h,
        "c18_file_menu",
        Region {
            labels: &["データを開く…", "終了"],
            pad: 8.0,
            x0: Some(0.0),
            to_right_edge: false,
        },
    );
}
