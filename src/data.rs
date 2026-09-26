//! Reading columnar text data (CSV, TSV, whitespace-separated `.dat`/`.txt`) and
//! JASCO spectrometer exports.
//!
//! The generic parser is deliberately forgiving: comment lines (`#`, `%`, `;`,
//! `!`) and any non-numeric lines are skipped, the last non-numeric line before
//! the data becomes the column names, and short rows are padded with NaN.
//!
//! JASCO files (`DATA TYPE` / `XYDATA` headers, as written by Spectra Manager)
//! are recognised by content, whatever their extension. A 2-D export (e.g. a
//! temperature scan) becomes one table: the first column is wavelength and every
//! other column is one spectrum, headed by its temperature.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub columns: Vec<Vec<f64>>,
}

impl Table {
    pub fn rows(&self) -> usize {
        self.columns.first().map_or(0, Vec::len)
    }
}

fn split_fields(line: &str) -> Vec<&str> {
    let parts: Vec<&str> = if line.contains(',') {
        line.split(',').collect()
    } else if line.contains('\t') {
        line.split('\t').collect()
    } else if line.contains(';') {
        line.split(';').collect()
    } else {
        line.split_whitespace().collect()
    };
    let mut parts: Vec<&str> = parts.into_iter().map(str::trim).collect();
    // A trailing delimiter should not create an extra empty column.
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

fn parse_number(s: &str) -> Option<f64> {
    if s.is_empty() {
        return Some(f64::NAN);
    }
    let s = s.trim_matches('"');
    match s.to_ascii_lowercase().as_str() {
        "nan" | "na" | "n/a" => Some(f64::NAN),
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        _ => s.parse().ok(),
    }
}

/// JASCO Spectra Manager text/CSV export, if `text` is one.
fn parse_jasco(text: &str) -> Option<Result<Table, String>> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let start = lines
        .iter()
        .position(|l| l.trim().trim_end_matches(',') == "XYDATA")?;
    if !lines[..start].iter().any(|l| l.starts_with("DATA TYPE")) {
        return None;
    }
    // CSV exports use commas where the text export uses tabs.
    let sep = if lines[start + 1..].iter().take(3).any(|l| l.contains('\t')) {
        '\t'
    } else {
        ','
    };
    let header = |key: &str| {
        lines[..start].iter().find_map(|l| {
            let rest = l.strip_prefix(key)?;
            let v = rest
                .trim_start_matches([',', '\t', ' '])
                .trim()
                .trim_end_matches(',');
            (!v.is_empty()).then(|| v.to_string())
        })
    };
    let unit_name = |u: Option<String>, fallback: &str| match u.as_deref() {
        Some("NANOMETERS") => "wavelength (nm)".to_string(),
        Some("1/CM") => "wavenumber (1/cm)".to_string(),
        Some(u) => u.to_lowercase(),
        None => fallback.to_string(),
    };
    let x_name = unit_name(header("XUNITS"), "x");
    let y_name = unit_name(header("YUNITS"), "y");

    let block: Vec<&str> = lines[start + 1..]
        .iter()
        .copied()
        .take_while(|l| !l.trim().is_empty())
        .collect();
    let Some(first) = block.first() else {
        return Some(Err("JASCO file has no data".into()));
    };
    let fields = |l: &str| -> Vec<String> { l.split(sep).map(|f| f.trim().to_string()).collect() };
    let first_fields = fields(first);
    // 2-D data: the first row is "<empty>, series values..." (e.g. temperatures).
    let two_d = first_fields.first().is_some_and(|f| f.is_empty());
    let (headers, body): (Vec<String>, &[&str]) = if two_d {
        let mut h = vec![x_name];
        h.extend(first_fields[1..].iter().filter(|f| !f.is_empty()).cloned());
        (h, &block[1..])
    } else {
        (vec![x_name, y_name], &block[..])
    };
    let mut columns = vec![Vec::with_capacity(body.len()); headers.len()];
    for line in body {
        let f = fields(line);
        let Some(x) = f.first().and_then(|v| v.parse::<f64>().ok()) else {
            continue;
        };
        columns[0].push(x);
        for (c, col) in columns.iter_mut().enumerate().skip(1) {
            col.push(f.get(c).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN));
        }
    }
    if columns[0].is_empty() {
        return Some(Err("JASCO file has no numeric data".into()));
    }
    Some(Ok(Table { headers, columns }))
}

pub fn parse_table(text: &str) -> Result<Table, String> {
    if let Some(t) = parse_jasco(text) {
        return t;
    }
    let mut header: Option<Vec<String>> = None;
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let comment = line.starts_with(['#', '%', ';', '!']);
        let body = line.trim_start_matches(['#', '%', ';', '!']).trim();
        let fields = split_fields(body);
        if fields.is_empty() {
            continue;
        }
        if !comment
            && let Some(nums) = fields
                .iter()
                .map(|f| parse_number(f))
                .collect::<Option<Vec<f64>>>()
            && nums.iter().any(|v| !v.is_nan())
        {
            rows.push(nums);
            continue;
        }
        if rows.is_empty() {
            header = Some(
                fields
                    .iter()
                    .map(|s| s.trim_matches('"').to_string())
                    .collect(),
            );
        }
    }
    if rows.is_empty() {
        return Err("no numeric data found".into());
    }
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut columns = vec![Vec::with_capacity(rows.len()); ncols];
    for r in &rows {
        for (c, col) in columns.iter_mut().enumerate() {
            col.push(r.get(c).copied().unwrap_or(f64::NAN));
        }
    }
    let headers = match header {
        Some(h) if h.len() == ncols => h,
        _ => (1..=ncols).map(|i| format!("col{i}")).collect(),
    };
    Ok(Table { headers, columns })
}

pub fn load_table(path: &Path) -> Result<Table, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_table(&String::from_utf8_lossy(&bytes))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weighting {
    /// Use the σ column when one is chosen, otherwise unit weights.
    #[default]
    Sigma,
    /// Ignore σ; every point counts the same.
    Unit,
    /// Counting statistics: σ = sqrt(|y|).
    Poisson,
    /// σ proportional to y, which suits data spanning decades (e.g. SAS on log axes).
    Relative,
}

impl Weighting {
    pub const ALL: [Weighting; 4] = [
        Weighting::Sigma,
        Weighting::Unit,
        Weighting::Poisson,
        Weighting::Relative,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Weighting::Sigma => "σ column",
            Weighting::Unit => "none (unit)",
            Weighting::Poisson => "Poisson √y",
            Weighting::Relative => "relative (σ ∝ y)",
        }
    }
}

/// A table plus the choice of which columns are x, y and σ.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Dataset {
    pub name: String,
    pub path: Option<PathBuf>,
    pub table: Table,
    pub x_col: usize,
    pub y_col: usize,
    pub sigma_col: Option<usize>,
    pub weighting: Weighting,
    /// Only points with x inside this range enter the fit.
    pub range: Option<(f64, f64)>,
    /// Named numbers describing the sample (e.g. `conc_M`), usable in constraints
    /// as `conc_M` or `D1.conc_M`. Filled from the file name on load.
    pub constants: BTreeMap<String, f64>,
}

/// The number at the end of `s`, e.g. `50` or `12.5`, including the
/// file-name-safe decimal spellings `62dot5` and `12p5`.
fn trailing_number(s: &str) -> Option<f64> {
    let digits_start = |t: &str| t.len() - t.bytes().rev().take_while(u8::is_ascii_digit).count();
    let i = digits_start(s);
    let frac = &s[i..];
    if frac.is_empty() {
        return None;
    }
    let head = &s[..i];
    for sep in [".", "dot", "p"] {
        if let Some(h) = head.strip_suffix(sep) {
            let j = digits_start(h);
            if j < h.len() {
                return format!("{}.{frac}", &h[j..]).parse().ok();
            }
        }
    }
    frac.parse().ok()
}

/// Numbers encoded in a file name, as constraint-ready constants:
/// - a concentration like `_50microM_`, `12.5uM`, `62dot5µM` or `2mM` gives
///   `conc_uM` and `conc_M`;
/// - tokens like `THF200`, `Water300`, `EtOH10` give `THF = 200`, `Water = 300`, ...
pub fn constants_from_name(name: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    // Stop at the first '.' that starts an extension (`.txt`, `.txt.re`), but keep
    // decimal points such as `0.5mM`.
    let stem = match name.find(".txt").or_else(|| name.find(".csv")) {
        Some(i) => &name[..i],
        None => name.rsplit_once('.').map_or(name, |(a, _)| a),
    };
    'units: for (unit, to_um) in [("microM", 1.0), ("uM", 1.0), ("µM", 1.0), ("mM", 1000.0)] {
        let mut from = 0;
        while let Some(pos) = stem[from..].find(unit).map(|p| p + from) {
            from = pos + unit.len();
            // The unit must end its token, and something must precede it.
            if stem[from..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
                || pos == 0
            {
                continue;
            }
            if let Some(v) = trailing_number(&stem[..pos]) {
                out.insert("conc_uM".into(), v * to_um);
                out.insert("conc_M".into(), v * to_um * 1e-6);
                break 'units;
            }
        }
    }
    for tok in stem.split(['_', ' ', '-']) {
        let split = tok.find(|c: char| c.is_ascii_digit()).unwrap_or(tok.len());
        let (letters, number) = tok.split_at(split);
        if letters.len() >= 2
            && letters.chars().all(|c| c.is_ascii_alphabetic())
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit() || c == '.')
            && let Ok(v) = number.parse::<f64>()
        {
            out.insert(letters.to_string(), v);
        }
    }
    out
}

/// The points that enter a fit, with weights `1/σ`.
#[derive(Clone, Debug, Default)]
pub struct FitArrays {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub w: Vec<f64>,
}

impl Dataset {
    pub fn from_table(name: String, path: Option<PathBuf>, table: Table) -> Self {
        let n = table.columns.len();
        // SAS-style three-column files (q, I, dI) and anything whose header says so get σ.
        let sigma_col = (2..n).find(|&i| {
            let h = table.headers[i].to_ascii_lowercase();
            ["err", "sig", "std", "unc"].iter().any(|k| h.contains(k))
                || ["di", "dy", "sd", "dyi"].contains(&h.as_str())
        });
        Self {
            name,
            path,
            x_col: 0,
            y_col: usize::from(n > 1),
            sigma_col,
            table,
            weighting: Weighting::Sigma,
            range: None,
            constants: BTreeMap::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let table = load_table(path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "data".into());
        let mut d = Self::from_table(name, Some(path.to_path_buf()), table);
        d.constants = constants_from_name(&d.name);
        Ok(d)
    }

    fn col(&self, i: usize) -> &[f64] {
        self.table.columns.get(i).map_or(&[], Vec::as_slice)
    }

    pub fn x(&self) -> &[f64] {
        self.col(self.x_col)
    }

    pub fn y(&self) -> &[f64] {
        self.col(self.y_col)
    }

    pub fn sigma(&self) -> Option<&[f64]> {
        self.sigma_col.map(|c| self.col(c))
    }

    pub fn in_range(&self, x: f64) -> bool {
        match self.range {
            Some((lo, hi)) => x >= lo.min(hi) && x <= hi.max(lo),
            None => true,
        }
    }

    pub fn x_extent(&self) -> Option<(f64, f64)> {
        let mut it = self.x().iter().copied().filter(|v| v.is_finite());
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
    }

    /// Points inside the fit range with finite x, y and a usable weight.
    pub fn fit_arrays(&self) -> FitArrays {
        let (x, y) = (self.x(), self.y());
        let sigma = self.sigma();
        let mut out = FitArrays::default();
        for i in 0..x.len().min(y.len()) {
            let (xi, yi) = (x[i], y[i]);
            if !xi.is_finite() || !yi.is_finite() || !self.in_range(xi) {
                continue;
            }
            let s = match self.weighting {
                Weighting::Sigma => sigma.map_or(1.0, |s| s.get(i).copied().unwrap_or(f64::NAN)),
                Weighting::Unit => 1.0,
                Weighting::Poisson => yi.abs().sqrt().max(1.0),
                Weighting::Relative => yi.abs(),
            };
            if !(s.is_finite() && s > 0.0) {
                continue;
            }
            out.x.push(xi);
            out.y.push(yi);
            out.w.push(1.0 / s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JASCO_1D: &str = "TITLE\t\r\nDATA TYPE\tULTRAVIOLET SPECTRUM\r\nXUNITS\tNANOMETERS\r\nYUNITS\tABSORBANCE\r\nXYDATA\r\n500.0000\t0.0186\r\n499.5000\t0.0187\r\n\r\n[Comments]\r\nNo. of cycles\t1\r\n";
    const JASCO_2D: &str = "DATA TYPE\tULTRAVIOLET SPECTRUM\nXUNITS\tNANOMETERS\nXYDATA\n\t20.01\t22.02\t24\n500\t0.1\t0.2\t0.3\n499.5\t0.11\t0.21\t\n\n";

    #[test]
    fn jasco_single_spectrum() {
        let t = parse_table(JASCO_1D).unwrap();
        assert_eq!(t.headers, ["wavelength (nm)", "absorbance"]);
        assert_eq!(t.columns[0], [500.0, 499.5]);
        assert_eq!(t.columns[1], [0.0186, 0.0187]);
    }

    #[test]
    fn jasco_temperature_scan() {
        let t = parse_table(JASCO_2D).unwrap();
        assert_eq!(t.headers, ["wavelength (nm)", "20.01", "22.02", "24"]);
        assert_eq!(t.columns[3][0], 0.3);
        assert!(t.columns[3][1].is_nan());
        // The CSV export of the same data.
        let csv = JASCO_2D.replace('\t', ",");
        // (NaN != NaN, so compare the printed form.)
        assert_eq!(
            format!("{:?}", parse_table(&csv).unwrap()),
            format!("{t:?}")
        );
    }

    #[test]
    fn constants_from_file_names() {
        let c = constants_from_name("20251015_iQuinStilNaphOMeTTP_50microM_THF200_Water300.txt");
        assert_eq!(c["conc_uM"], 50.0);
        assert!((c["conc_M"] - 5e-5).abs() < 1e-18);
        assert_eq!((c["THF"], c["Water"]), (200.0, 300.0));
        let c = constants_from_name("x_final62dot5microM_Acetone400_Water0.txt");
        assert_eq!(c["conc_uM"], 62.5);
        assert_eq!(c["Acetone"], 400.0);
        let c = constants_from_name("QuinNaphtTDP_0.5mM_MCH_0.1Cooling.txt.re");
        assert_eq!(c["conc_uM"], 500.0);
        assert!(!c.contains_key("MCH"));
        let c = constants_from_name("iQuinNaphTDP_100microM_MCH.txt");
        assert_eq!(c["conc_uM"], 100.0);
        assert!(constants_from_name("plain.csv").is_empty());
    }

    #[test]
    fn csv_with_header() {
        let t = parse_table("x,y,err\n1,2,0.1\n2,4,0.2\n").unwrap();
        assert_eq!(t.headers, ["x", "y", "err"]);
        assert_eq!(t.columns[1], [2.0, 4.0]);
        let d = Dataset::from_table("t".into(), None, t);
        assert_eq!((d.x_col, d.y_col, d.sigma_col), (0, 1, Some(2)));
    }

    #[test]
    fn whitespace_with_comment_header_and_junk() {
        let text = "# SAS data\n# q I dI\n0.01  100  5\n0.02\t80\t4\n\nEND\n0.03 60 3.0e0\n";
        let t = parse_table(text).unwrap();
        assert_eq!(t.headers, ["q", "I", "dI"]);
        assert_eq!(t.rows(), 3);
        assert_eq!(t.columns[2][2], 3.0);
    }

    #[test]
    fn ragged_rows_are_padded() {
        let t = parse_table("1 2 3\n4 5\n").unwrap();
        assert_eq!(t.columns.len(), 3);
        assert!(t.columns[2][1].is_nan());
        assert_eq!(t.headers, ["col1", "col2", "col3"]);
    }

    #[test]
    fn semicolons_and_trailing_delimiters() {
        let t = parse_table("a;b;\n1;2;\n3;4;\n").unwrap();
        assert_eq!(t.headers, ["a", "b"]);
        assert_eq!(t.columns[0], [1.0, 3.0]);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(parse_table("# nothing\nhello\n").is_err());
    }

    #[test]
    fn fit_arrays_apply_range_and_weights() {
        let t = parse_table("x y s\n0 1 0.5\n1 4 0\n2 9 1\n3 nan 1\n").unwrap();
        let mut d = Dataset::from_table("t".into(), None, t);
        d.sigma_col = Some(2);
        let a = d.fit_arrays();
        // σ = 0 and y = NaN rows are dropped.
        assert_eq!(a.x, [0.0, 2.0]);
        assert_eq!(a.w, [2.0, 1.0]);
        d.weighting = Weighting::Unit;
        d.range = Some((0.5, 5.0));
        let a = d.fit_arrays();
        assert_eq!(a.x, [1.0, 2.0]);
        assert_eq!(a.w, [1.0, 1.0]);
    }
}
