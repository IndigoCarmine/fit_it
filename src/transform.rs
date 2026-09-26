//! Turning spectra into fit-ready curves ("cross-sections").
//!
//! Typical UV-Vis workflows:
//! - a temperature scan (one JASCO 2-D file: wavelength × temperature) becomes
//!   absorbance-at-λ vs temperature — use [`slice_columns`];
//! - a titration (one spectrum per file, concentration in the file name) becomes
//!   absorbance-at-λ vs concentration — use [`slice_across`].
//!
//! Per spectrum: subtract a baseline (mean over an x-range), read the value at λ
//! (nearest point, or the mean within ±window), optionally divide by a dataset
//! constant (e.g. `conc_uM`, for ε-like curves). Per curve: sort by x, apply
//! `x * x_scale + x_offset` (e.g. °C → K), then optionally normalise to 0…1 and
//! invert (`1 - y`).

use crate::data::{Dataset, Table};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct SliceOptions {
    /// x position to read (e.g. wavelength in nm).
    pub at: f64,
    /// Average over |x - at| <= window; 0 means the nearest point.
    pub window: f64,
    /// Subtract the mean over this x-range from each spectrum first.
    pub baseline: Option<(f64, f64)>,
    /// Divide each value by this dataset constant.
    pub divide_by: Option<String>,
    pub x_scale: f64,
    pub x_offset: f64,
    pub normalize: bool,
    pub invert: bool,
    pub x_label: String,
}

impl Default for SliceOptions {
    fn default() -> Self {
        Self {
            at: 0.0,
            window: 0.0,
            baseline: None,
            divide_by: None,
            x_scale: 1.0,
            x_offset: 0.0,
            normalize: false,
            invert: false,
            x_label: "x".into(),
        }
    }
}

fn finite_pairs<'a>(x: &'a [f64], y: &'a [f64]) -> impl Iterator<Item = (f64, f64)> + 'a {
    x.iter()
        .zip(y)
        .map(|(a, b)| (*a, *b))
        .filter(|(a, b)| a.is_finite() && b.is_finite())
}

/// Mean of y over the finite points whose x satisfies `keep`.
fn mean_where(x: &[f64], y: &[f64], keep: impl Fn(f64) -> bool) -> Option<f64> {
    let (sum, n) = finite_pairs(x, y)
        .filter(|(a, _)| keep(*a))
        .fold((0.0, 0usize), |(s, n), (_, b)| (s + b, n + 1));
    (n > 0).then(|| sum / n as f64)
}

/// Value of the spectrum `(x, y)` at `at`: nearest point, or the mean within ±window.
pub fn value_at(x: &[f64], y: &[f64], at: f64, window: f64) -> Option<f64> {
    if window > 0.0 {
        return mean_where(x, y, |a| (a - at).abs() <= window);
    }
    finite_pairs(x, y)
        .min_by(|p, q| (p.0 - at).abs().total_cmp(&(q.0 - at).abs()))
        .map(|p| p.1)
}

/// Mean of y over x in `[lo, hi]` (either order).
pub fn range_mean(x: &[f64], y: &[f64], (lo, hi): (f64, f64)) -> Option<f64> {
    let (lo, hi) = (lo.min(hi), hi.max(lo));
    mean_where(x, y, |a| a >= lo && a <= hi)
}

fn read(x: &[f64], y: &[f64], o: &SliceOptions, divisor: f64) -> Result<f64, String> {
    let base = match o.baseline {
        Some(r) => range_mean(x, y, r)
            .ok_or_else(|| format!("no points in the baseline range {}–{}", r.0, r.1))?,
        None => 0.0,
    };
    let v = value_at(x, y, o.at, o.window).ok_or_else(|| format!("no data at x = {}", o.at))?;
    Ok((v - base) / divisor)
}

fn divisor(d: &Dataset, o: &SliceOptions) -> Result<f64, String> {
    match &o.divide_by {
        None => Ok(1.0),
        Some(name) => match d.constants.get(name) {
            Some(v) if *v != 0.0 => Ok(*v),
            Some(_) => Err(format!("{}: {name} is 0, cannot divide by it", d.name)),
            None => Err(format!("{}: has no constant `{name}`", d.name)),
        },
    }
}

fn finish(mut pts: Vec<(f64, f64)>, o: &SliceOptions) -> Result<Vec<(f64, f64)>, String> {
    if pts.is_empty() {
        return Err("nothing to slice".into());
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    for p in &mut pts {
        p.0 = p.0 * o.x_scale + o.x_offset;
    }
    if o.normalize {
        let lo = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let hi = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        if hi - lo <= 0.0 {
            return Err("cannot normalise: all values are equal".into());
        }
        for p in &mut pts {
            p.1 = (p.1 - lo) / (hi - lo);
        }
    }
    if o.invert {
        for p in &mut pts {
            p.1 = 1.0 - p.1;
        }
    }
    Ok(pts)
}

fn to_dataset(
    name: String,
    pts: Vec<(f64, f64)>,
    o: &SliceOptions,
    constants: BTreeMap<String, f64>,
) -> Dataset {
    let table = Table {
        headers: vec![o.x_label.clone(), format!("y @ {}", o.at)],
        columns: vec![
            pts.iter().map(|p| p.0).collect(),
            pts.iter().map(|p| p.1).collect(),
        ],
    };
    let mut d = Dataset::from_table(name, None, table);
    d.constants = constants;
    d
}

/// Columns of a multi-spectrum table whose headers are numbers (e.g. temperatures).
pub fn series_columns(d: &Dataset) -> Vec<(usize, f64)> {
    d.table
        .headers
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != d.x_col && Some(*i) != d.sigma_col)
        .filter_map(|(i, h)| h.trim().parse::<f64>().ok().map(|v| (i, v)))
        .collect()
}

/// Value at `o.at` of every spectrum column of `d`, against its header value
/// (e.g. temperature). The result keeps `d`'s constants.
pub fn slice_columns(d: &Dataset, o: &SliceOptions) -> Result<Dataset, String> {
    let cols = series_columns(d);
    if cols.is_empty() {
        return Err(format!(
            "{}: no columns with numeric headers (expected e.g. a JASCO temperature scan)",
            d.name
        ));
    }
    let div = divisor(d, o)?;
    let x = d.x();
    let pts = cols
        .iter()
        .map(|&(c, v)| Ok((v, read(x, &d.table.columns[c], o, div)?)))
        .collect::<Result<Vec<_>, String>>()
        .map_err(|e| format!("{}: {e}", d.name))?;
    let pts = finish(pts, o)?;
    Ok(to_dataset(
        format!("{} @ {}", d.name, o.at),
        pts,
        o,
        d.constants.clone(),
    ))
}

/// One point per dataset: the value at `o.at` of its y column, against its
/// constant `x_constant` (e.g. `conc_uM` in a titration).
pub fn slice_across(
    ds: &[&Dataset],
    x_constant: &str,
    o: &SliceOptions,
) -> Result<Dataset, String> {
    let pts = ds
        .iter()
        .map(|d| {
            let xv = *d
                .constants
                .get(x_constant)
                .ok_or_else(|| format!("{}: has no constant `{x_constant}`", d.name))?;
            let v =
                read(d.x(), d.y(), o, divisor(d, o)?).map_err(|e| format!("{}: {e}", d.name))?;
            Ok((xv, v))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let pts = finish(pts, o)?;
    // Keep only the constants every source agrees on (e.g. the solvent ratio).
    let mut shared = ds.first().map(|d| d.constants.clone()).unwrap_or_default();
    shared.retain(|k, v| k != x_constant && ds.iter().all(|d| d.constants.get(k) == Some(v)));
    Ok(to_dataset(
        format!("{x_constant} series @ {}", o.at),
        pts,
        o,
        shared,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::parse_table;

    fn scan() -> Dataset {
        // Two wavelengths, three temperatures; baseline at 500 nm.
        let text = "DATA TYPE\tULTRAVIOLET SPECTRUM\nXUNITS\tNANOMETERS\nXYDATA\n\t20\t40\t60\n500\t0.02\t0.01\t0.03\n400\t1.02\t0.51\t0.13\n\n";
        let mut d =
            Dataset::from_table("scan_50microM.txt".into(), None, parse_table(text).unwrap());
        d.constants = crate::data::constants_from_name(&d.name);
        d
    }

    #[test]
    fn temperature_scan_to_curve() {
        let o = SliceOptions {
            at: 401.0,
            baseline: Some((450.0, 600.0)),
            x_offset: 273.15,
            normalize: true,
            ..Default::default()
        };
        let d = slice_columns(&scan(), &o).unwrap();
        assert_eq!(d.x(), [293.15, 313.15, 333.15]);
        // Baseline-corrected: 1.00, 0.50, 0.10 → normalised.
        let y = d.y();
        assert!(
            (y[0] - 1.0).abs() < 1e-12 && (y[1] - 0.4 / 0.9).abs() < 1e-12 && y[2].abs() < 1e-12
        );
        assert_eq!(
            d.constants["conc_uM"], 50.0,
            "constants carry over for c_tot"
        );

        let inv = slice_columns(&scan(), &SliceOptions { invert: true, ..o }).unwrap();
        assert!((inv.y()[0]).abs() < 1e-12);
    }

    #[test]
    fn titration_across_files() {
        let mk = |c: f64, a: f64| {
            let t = parse_table(&format!("x y\n400 {a}\n450 0.01\n")).unwrap();
            let mut d = Dataset::from_table(format!("s_{c}microM_THF200.txt"), None, t);
            d.constants = crate::data::constants_from_name(&d.name);
            d
        };
        let ds = [mk(20.0, 0.41), mk(10.0, 0.21)];
        let refs: Vec<&Dataset> = ds.iter().collect();
        let o = SliceOptions {
            at: 400.0,
            baseline: Some((450.0, 450.0)),
            divide_by: Some("conc_uM".into()),
            x_scale: 1e-6,
            ..Default::default()
        };
        let out = slice_across(&refs, "conc_uM", &o).unwrap();
        assert!((out.x()[0] - 1e-5).abs() < 1e-18 && (out.x()[1] - 2e-5).abs() < 1e-18);
        assert!((out.y()[0] - 0.02).abs() < 1e-12 && (out.y()[1] - 0.02).abs() < 1e-12);
        assert_eq!(out.constants.get("THF"), Some(&200.0));
        assert!(!out.constants.contains_key("conc_uM"));
        assert!(slice_across(&refs, "missing", &o).is_err());
    }

    #[test]
    fn window_average_and_errors() {
        let x = [1.0, 2.0, 3.0];
        let y = [1.0, 2.0, 6.0];
        assert_eq!(value_at(&x, &y, 2.2, 0.0), Some(2.0));
        assert_eq!(value_at(&x, &y, 2.0, 1.0), Some(3.0));
        let flat = SliceOptions {
            at: 400.0,
            normalize: true,
            ..Default::default()
        };
        let mut d = scan();
        d.table.columns[1][1] = 0.5;
        d.table.columns[2][1] = 0.5;
        d.table.columns[3][1] = 0.5;
        assert!(slice_columns(&d, &flat).is_err());
    }
}
