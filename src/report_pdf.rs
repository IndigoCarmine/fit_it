//! PDF fit reports, written directly (no PDF library): A4 pages with vector
//! plots of every fitted dataset, parameter tables and the lmfit-style report.
//!
//! Text uses the bundled Noto Sans JP (SIL Open Font License), embedded as a
//! subset holding only the glyphs the report uses, so Japanese file names and
//! symbols print correctly while the file stays small. The monospaced report
//! and tables use the standard Courier font (not embedded) when they are plain
//! WinAnsi text, which keeps their columns aligned.

use crate::export::{DatasetCurves, PALETTE};
use crate::fit::Param;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::rc::Rc;

/// Noto Sans JP Regular (SIL Open Font License 1.1; see resources/fonts/OFL.txt).
/// Also the app's UI font for Japanese.
pub const NOTO_SANS_JP: &[u8] = include_bytes!("../resources/fonts/NotoSansJP-Regular.otf");
const NOTO_NAME: &str = "NotoSansJP-Regular";

/// The embedded font as the report is written: glyphs are renumbered on first
/// use (the numbers go straight into the page content) and subset at the end.
struct Embedded {
    face: ttf_parser::Face<'static>,
    remap: subsetter::GlyphRemapper,
    /// New glyph id -> (advance in 1/1000 em, text it stands for).
    glyphs: BTreeMap<u16, (u32, String)>,
}

impl Embedded {
    fn new() -> Self {
        Self {
            face: ttf_parser::Face::parse(NOTO_SANS_JP, 0).expect("bundled font parses"),
            remap: subsetter::GlyphRemapper::new(),
            glyphs: BTreeMap::new(),
        }
    }

    fn scale(&self) -> f64 {
        1000.0 / f64::from(self.face.units_per_em())
    }

    /// Hex string of new glyph ids for `s` (Identity-H), and its width in 1/1000 em.
    fn encode(&mut self, s: &str) -> (String, f64) {
        let mut hex = String::new();
        let mut width = 0.0;
        for ch in s.chars() {
            let old = self.face.glyph_index(ch).map_or(0, |g| g.0);
            let adv = self
                .face
                .glyph_hor_advance(ttf_parser::GlyphId(old))
                .unwrap_or(0);
            let w = (f64::from(adv) * self.scale()).round() as u32;
            let new = self.remap.remap(old);
            self.glyphs
                .entry(new)
                .or_insert_with(|| (w, ch.to_string()));
            let _ = write!(hex, "{new:04X}");
            width += f64::from(w);
        }
        (hex, width)
    }

    fn width(&self, s: &str) -> f64 {
        s.chars()
            .map(|ch| {
                let g = self.face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                f64::from(self.face.glyph_hor_advance(g).unwrap_or(0)) * self.scale()
            })
            .sum()
    }
}

type FontRef = Rc<RefCell<Embedded>>;

const PAGE_W: f64 = 595.0;
const PAGE_H: f64 = 842.0;

#[derive(Clone, Copy, PartialEq)]
enum Font {
    Regular,
    /// The regular face drawn with a thin outline (Noto Sans JP is bundled in
    /// one weight only).
    Bold,
    Mono,
}

type Rgb = (f64, f64, f64);

fn rgb((r, g, b): (u8, u8, u8)) -> Rgb {
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

const BLACK: Rgb = (0.0, 0.0, 0.0);
const GREY: Rgb = (0.55, 0.55, 0.55);
const LIGHT: Rgb = (0.88, 0.88, 0.88);

/// Can `s` be printed in a standard (WinAnsi) font without substitutions?
fn winansi(s: &str) -> bool {
    s.chars()
        .all(|c| (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c))
}

/// WinAnsi-encode `s` as a PDF string literal body.
fn pdf_text(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        let spelled = match ch {
            'χ' => Some("chi"),
            'σ' => Some("sigma"),
            'Δ' => Some("Delta"),
            'ε' => Some("eps"),
            '−' => Some("-"),
            '→' => Some("->"),
            _ => None,
        };
        if let Some(sp) = spelled {
            out.push_str(sp);
            continue;
        }
        let byte: Option<u8> = match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
                continue;
            }
            c if (' '..='~').contains(&c) => {
                out.push(c);
                continue;
            }
            '—' => Some(0x97),
            '–' => Some(0x96),
            '…' => Some(0x85),
            '•' => Some(0x95),
            c if ('\u{a0}'..='\u{ff}').contains(&c) => Some(c as u32 as u8),
            _ => None,
        };
        match byte {
            Some(b) => {
                let _ = write!(out, "\\{b:03o}");
            }
            None => out.push('?'),
        }
    }
    out
}

/// Compact number for tick labels and tables.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    let a = v.abs();
    if a == 0.0 || (1e-3..1e5).contains(&a) {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.to_string() }
    } else {
        let s = format!("{v:.3e}");
        match s.split_once('e') {
            Some((m, e)) => format!("{}e{e}", m.trim_end_matches('0').trim_end_matches('.')),
            None => s,
        }
    }
}

struct Page {
    ops: String,
    font: FontRef,
}

impl Page {
    fn new(font: &FontRef) -> Self {
        Self {
            ops: String::new(),
            font: font.clone(),
        }
    }

    /// Courier for plain monospaced text; the embedded font for everything else.
    fn use_courier(font: Font, s: &str) -> bool {
        font == Font::Mono && winansi(s)
    }

    fn width(&self, s: &str, size: f64, font: Font) -> f64 {
        if Self::use_courier(font, s) {
            s.chars().count() as f64 * size * 0.6
        } else {
            self.font.borrow().width(s) * size / 1000.0
        }
    }

    fn color(&mut self, (r, g, b): Rgb, stroke: bool) {
        let op = if stroke { "RG" } else { "rg" };
        let _ = writeln!(self.ops, "{r:.3} {g:.3} {b:.3} {op}");
    }

    /// Text-showing operators for `s` at the text matrix `tm`.
    fn show(&mut self, tm: &str, size: f64, font: Font, s: &str) {
        self.color(BLACK, false);
        if Self::use_courier(font, s) {
            let _ = writeln!(self.ops, "BT /F1 {size} Tf {tm} Tm ({}) Tj ET", pdf_text(s));
            return;
        }
        let (hex, _) = self.font.borrow_mut().encode(s);
        if font == Font::Bold {
            // Fill + thin stroke: a faux bold that stays crisp at any zoom.
            self.color(BLACK, true);
            let _ = writeln!(
                self.ops,
                "BT 2 Tr {:.3} w /F2 {size} Tf {tm} Tm <{hex}> Tj 0 Tr ET",
                size * 0.035
            );
        } else {
            let _ = writeln!(self.ops, "BT /F2 {size} Tf {tm} Tm <{hex}> Tj ET");
        }
    }

    fn text(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        self.show(&format!("1 0 0 1 {x:.2} {y:.2}"), size, font, s);
    }

    fn text_centered(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        let w = self.width(s, size, font);
        self.text(x - w / 2.0, y, size, font, s);
    }

    fn text_right(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        let w = self.width(s, size, font);
        self.text(x - w, y, size, font, s);
    }

    /// Text rotated 90° counter-clockwise, centred on (x, y).
    fn text_vertical(&mut self, x: f64, y: f64, size: f64, s: &str) {
        let w = self.width(s, size, Font::Regular);
        self.show(
            &format!("0 1 -1 0 {x:.2} {:.2}", y - w / 2.0),
            size,
            Font::Regular,
            s,
        );
    }

    fn line_style(&mut self, width: f64, dash: Option<(f64, f64)>) {
        let _ = writeln!(self.ops, "{width:.2} w");
        match dash {
            Some((a, b)) => {
                let _ = writeln!(self.ops, "[{a} {b}] 0 d");
            }
            None => self.ops.push_str("[] 0 d\n"),
        }
    }

    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        let _ = writeln!(self.ops, "{x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S");
    }

    fn polyline(&mut self, pts: &[(f64, f64)]) {
        // Break at non-finite points (e.g. log of a non-positive value).
        let mut open = false;
        for &(x, y) in pts {
            if !(x.is_finite() && y.is_finite()) {
                if open {
                    self.ops.push_str("S\n");
                }
                open = false;
                continue;
            }
            let _ = writeln!(self.ops, "{x:.2} {y:.2} {}", if open { "l" } else { "m" });
            open = true;
        }
        if open {
            self.ops.push_str("S\n");
        }
    }

    fn circle(&mut self, x: f64, y: f64, r: f64) {
        let k = 0.5523 * r;
        let _ = writeln!(
            self.ops,
            "{:.2} {y:.2} m {:.2} {:.2} {:.2} {:.2} {x:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {y:.2} c \
             {:.2} {:.2} {:.2} {:.2} {x:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {y:.2} c S",
            x + r,
            x + r,
            y + k,
            x + k,
            y + r,
            y + r,
            x - k,
            y + r,
            x - r,
            y + k,
            x - r,
            x - r,
            y - k,
            x - k,
            y - r,
            y - r,
            x + k,
            y - r,
            x + r,
            y - k,
            x + r,
        );
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let _ = writeln!(self.ops, "{x:.2} {y:.2} {w:.2} {h:.2} re S");
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb) {
        self.color(c, false);
        let _ = writeln!(self.ops, "{x:.2} {y:.2} {w:.2} {h:.2} re f");
    }
}

// ---------------------------------------------------------------------------
// Plots

struct Axis {
    lo: f64,
    hi: f64,
    log: bool,
}

impl Axis {
    fn fit(values: impl Iterator<Item = f64>, log: bool, margin: f64) -> Axis {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for v in values {
            let v = if log {
                if v > 0.0 { v.log10() } else { continue }
            } else {
                v
            };
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if !lo.is_finite() {
            (lo, hi) = (0.0, 1.0);
        }
        if hi - lo <= 0.0 {
            let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
            (lo, hi) = (lo - pad, hi + pad);
        }
        let m = (hi - lo) * margin;
        Axis {
            lo: lo - m,
            hi: hi + m,
            log,
        }
    }

    fn tf(&self, v: f64) -> f64 {
        if self.log {
            if v > 0.0 { v.log10() } else { f64::NAN }
        } else {
            v
        }
    }

    /// Tick positions (in transformed units) with labels.
    fn ticks(&self) -> Vec<(f64, String)> {
        if self.log {
            let (a, b) = (self.lo.ceil() as i64, self.hi.floor() as i64);
            let step = ((b - a) / 6 + 1).max(1);
            return (a..=b)
                .step_by(step as usize)
                .map(|e| (e as f64, format!("1e{e}")))
                .collect();
        }
        let span = self.hi - self.lo;
        let raw = span / 6.0;
        let mag = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 2.5, 5.0, 10.0]
            .iter()
            .map(|m| m * mag)
            .find(|s| *s >= raw)
            .unwrap_or(10.0 * mag);
        let first = (self.lo / step).ceil() as i64;
        let last = (self.hi / step).floor() as i64;
        (first..=last)
            .map(|i| {
                let v = i as f64 * step;
                // Kill -0 and 1e-17-style round-off.
                let v = if v.abs() < step * 1e-9 { 0.0 } else { v };
                (v, num(v))
            })
            .collect()
    }
}

enum Mark {
    Points,
    Line {
        width: f64,
        dash: Option<(f64, f64)>,
    },
}

struct Series<'a> {
    x: &'a [f64],
    y: &'a [f64],
    color: Rgb,
    mark: Mark,
    label: Option<String>,
}

struct Frame {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[allow(clippy::too_many_arguments)]
fn draw_plot(
    page: &mut Page,
    f: &Frame,
    series: &[Series],
    xa: &Axis,
    ya: &Axis,
    x_label: Option<&str>,
    y_label: &str,
    zero_line: bool,
) {
    let px = |v: f64| f.x + (xa.tf(v) - xa.lo) / (xa.hi - xa.lo) * f.w;
    let py = |v: f64| f.y + (ya.tf(v) - ya.lo) / (ya.hi - ya.lo) * f.h;
    let tx = |t: f64| f.x + (t - xa.lo) / (xa.hi - xa.lo) * f.w;
    let ty = |t: f64| f.y + (t - ya.lo) / (ya.hi - ya.lo) * f.h;

    // Grid and ticks.
    page.line_style(0.4, None);
    for (t, label) in xa.ticks() {
        let x = tx(t);
        page.color(LIGHT, true);
        page.line(x, f.y, x, f.y + f.h);
        page.color(BLACK, true);
        page.line(x, f.y, x, f.y - 3.0);
        if x_label.is_some() {
            page.text_centered(x, f.y - 12.0, 7.5, Font::Regular, &label);
        }
    }
    for (t, label) in ya.ticks() {
        let y = ty(t);
        page.color(LIGHT, true);
        page.line(f.x, y, f.x + f.w, y);
        page.color(BLACK, true);
        page.line(f.x, y, f.x - 3.0, y);
        page.text_right(f.x - 5.0, y - 2.5, 7.5, Font::Regular, &label);
    }
    if zero_line && ya.lo < 0.0 && ya.hi > 0.0 && !ya.log {
        page.color(GREY, true);
        page.line_style(0.6, None);
        page.line(f.x, py(0.0), f.x + f.w, py(0.0));
    }

    // Data, clipped to the frame.
    page.ops.push_str("q\n");
    let _ = writeln!(
        page.ops,
        "{:.2} {:.2} {:.2} {:.2} re W n",
        f.x, f.y, f.w, f.h
    );
    for s in series {
        page.color(s.color, true);
        match s.mark {
            Mark::Points => {
                page.line_style(0.5, None);
                for (x, y) in s.x.iter().zip(s.y) {
                    let (a, b) = (px(*x), py(*y));
                    if a.is_finite() && b.is_finite() {
                        page.circle(a, b, 1.6);
                    }
                }
            }
            Mark::Line { width, dash } => {
                page.line_style(width, dash);
                let pts: Vec<(f64, f64)> =
                    s.x.iter().zip(s.y).map(|(x, y)| (px(*x), py(*y))).collect();
                page.polyline(&pts);
            }
        }
    }
    page.ops.push_str("Q\n");

    page.color(BLACK, true);
    page.line_style(0.8, None);
    page.rect(f.x, f.y, f.w, f.h);
    if let Some(xl) = x_label {
        page.text_centered(f.x + f.w / 2.0, f.y - 25.0, 8.5, Font::Regular, xl);
    }
    page.text_vertical(f.x - 42.0, f.y + f.h / 2.0, 8.5, y_label);

    // Legend, top right.
    let labelled: Vec<&Series> = series.iter().filter(|s| s.label.is_some()).collect();
    if !labelled.is_empty() {
        let width = labelled
            .iter()
            .map(|s| page.width(s.label.as_ref().unwrap(), 7.0, Font::Regular))
            .fold(0.0, f64::max)
            + 26.0;
        let (lx, mut ly) = (f.x + f.w - width - 6.0, f.y + f.h - 12.0);
        page.fill_rect(
            lx - 4.0,
            ly - (labelled.len() as f64 - 1.0) * 10.0 - 5.0,
            width + 6.0,
            labelled.len() as f64 * 10.0 + 4.0,
            (1.0, 1.0, 1.0),
        );
        for s in labelled {
            page.color(s.color, true);
            match s.mark {
                Mark::Points => {
                    page.line_style(0.5, None);
                    page.circle(lx + 8.0, ly + 2.5, 1.6);
                }
                Mark::Line { width, dash } => {
                    page.line_style(width, dash);
                    page.line(lx, ly + 2.5, lx + 16.0, ly + 2.5);
                }
            }
            page.text(lx + 20.0, ly, 7.0, Font::Regular, s.label.as_ref().unwrap());
            ly -= 10.0;
        }
    }
}

// ---------------------------------------------------------------------------
// Report

pub struct ReportInput<'a> {
    pub title: String,
    /// Lines under the title (fit label, time, project file, ...).
    pub info: Vec<String>,
    pub curves: &'a [DatasetCurves],
    /// Palette index of each curve (so colours match the app).
    pub colors: Vec<usize>,
    /// Parameters of each curve's dataset.
    pub params: Vec<Vec<Param>>,
    /// The lmfit-style report; empty when nothing has been fitted yet.
    pub report_text: &'a str,
    pub log_x: bool,
    pub log_y: bool,
}

fn dataset_series<'a>(
    c: &'a DatasetCurves,
    color: Rgb,
    components: bool,
    with_labels: bool,
) -> Vec<Series<'a>> {
    let mut s = Vec::new();
    let tag = &c.tag;
    s.push(Series {
        x: &c.x,
        y: &c.y,
        color,
        mark: Mark::Points,
        label: with_labels.then(|| format!("{tag} data")),
    });
    if components {
        for (i, (name, y)) in c.grid_components.iter().enumerate() {
            let shade = rgb(PALETTE[(i + 3) % PALETTE.len()]);
            s.push(Series {
                x: &c.grid_x,
                y,
                color: shade,
                mark: Mark::Line {
                    width: 0.8,
                    dash: Some((3.0, 2.0)),
                },
                label: with_labels.then(|| name.clone()),
            });
        }
    }
    if !c.grid_model.is_empty() {
        s.push(Series {
            x: &c.grid_x,
            y: &c.grid_model,
            color: (0.0, 0.0, 0.0),
            mark: Mark::Line {
                width: 1.2,
                dash: None,
            },
            label: with_labels.then(|| format!("{tag} fit")),
        });
    }
    s
}

fn y_values(c: &DatasetCurves) -> impl Iterator<Item = f64> + '_ {
    c.y.iter().chain(&c.grid_model).copied()
}

fn param_rows(params: &[Param]) -> Vec<String> {
    let w = params
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .max(9);
    let mut rows = vec![format!(
        "{:w$}  {:>14}  {:>14}  {}",
        "parameter", "value", "std. error", "note"
    )];
    for p in params {
        let err = p.stderr.map(num).unwrap_or_default();
        let note = if !p.expr.trim().is_empty() {
            format!("= {}", p.expr.trim())
        } else if !p.vary {
            "fixed".into()
        } else {
            let lo = if p.min.is_finite() {
                num(p.min)
            } else {
                "-inf".into()
            };
            let hi = if p.max.is_finite() {
                num(p.max)
            } else {
                "inf".into()
            };
            format!("[{lo}, {hi}]")
        };
        rows.push(format!(
            "{:w$}  {:>14}  {:>14}  {note}",
            p.name,
            num(p.value),
            err
        ));
    }
    rows
}

/// Mono text lines onto pages starting at `y`, adding pages as needed.
fn flow_mono(pages: &mut Vec<Page>, mut y: f64, lines: &[String], size: f64) {
    let step = size * 1.3;
    let max_chars = ((PAGE_W - 100.0) / (size * 0.6)) as usize;
    for line in lines {
        // Wrap long lines with a hanging indent.
        let chars: Vec<char> = line.chars().collect();
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < chars.len() || chunks.is_empty() {
            let width = if chunks.is_empty() {
                max_chars
            } else {
                max_chars - 4
            };
            let end = (start + width).min(chars.len());
            let chunk: String = chars[start..end].iter().collect();
            chunks.push(if start == 0 {
                chunk
            } else {
                format!("    {chunk}")
            });
            start = end;
            if start >= chars.len() {
                break;
            }
        }
        for chunk in chunks {
            if y < 50.0 {
                let font = pages.last().unwrap().font.clone();
                pages.push(Page::new(&font));
                y = PAGE_H - 60.0;
            }
            pages
                .last_mut()
                .unwrap()
                .text(50.0, y, size, Font::Mono, &chunk);
            y -= step;
        }
    }
}

pub fn render(input: &ReportInput) -> Vec<u8> {
    let font: FontRef = Rc::new(RefCell::new(Embedded::new()));
    let mut pages: Vec<Page> = Vec::new();
    let color = |i: usize| rgb(PALETTE[input.colors.get(i).copied().unwrap_or(i) % PALETTE.len()]);

    // Page 1: title, info, overview of all datasets.
    let mut p = Page::new(&font);
    p.text(50.0, PAGE_H - 60.0, 18.0, Font::Bold, &input.title);
    let mut y = PAGE_H - 82.0;
    for line in &input.info {
        p.text(50.0, y, 9.0, Font::Regular, line);
        y -= 13.0;
    }
    if !input.curves.is_empty() {
        let frame = Frame {
            x: 90.0,
            y: y - 330.0,
            w: 450.0,
            h: 300.0,
        };
        let xa = Axis::fit(
            input.curves.iter().flat_map(|c| c.x.iter().copied()),
            input.log_x,
            0.03,
        );
        let ya = Axis::fit(input.curves.iter().flat_map(y_values), input.log_y, 0.05);
        let mut series = Vec::new();
        for (i, c) in input.curves.iter().enumerate() {
            let col = color(i);
            series.push(Series {
                x: &c.x,
                y: &c.y,
                color: col,
                mark: Mark::Points,
                // Long file names would cover the data; the legend goes below.
                label: None,
            });
            if !c.grid_model.is_empty() {
                series.push(Series {
                    x: &c.grid_x,
                    y: &c.grid_model,
                    color: col,
                    mark: Mark::Line {
                        width: 1.2,
                        dash: None,
                    },
                    label: None,
                });
            }
        }
        let title = if input.curves.len() > 1 {
            "All datasets"
        } else {
            "Dataset"
        };
        p.text(frame.x, frame.y + frame.h + 8.0, 10.0, Font::Bold, title);
        let c0 = &input.curves[0];
        draw_plot(
            &mut p,
            &frame,
            &series,
            &xa,
            &ya,
            Some(&c0.x_label),
            &c0.y_label,
            false,
        );
        // Legend under the plot: data marker, fit line, name.
        let mut ly = frame.y - 50.0;
        for (i, c) in input.curves.iter().enumerate() {
            p.color(color(i), true);
            p.line_style(0.5, None);
            p.circle(frame.x + 4.0, ly + 2.5, 1.6);
            p.line_style(1.2, None);
            p.line(frame.x + 10.0, ly + 2.5, frame.x + 26.0, ly + 2.5);
            p.text(
                frame.x + 32.0,
                ly,
                8.0,
                Font::Regular,
                &format!("{}   {}", c.tag, c.name),
            );
            ly -= 12.0;
        }
    }
    pages.push(p);

    // One page per dataset: fit, residuals, parameters.
    for (i, c) in input.curves.iter().enumerate() {
        let mut p = Page::new(&font);
        p.text(
            50.0,
            PAGE_H - 55.0,
            13.0,
            Font::Bold,
            &format!("{}   {}", c.tag, c.name),
        );
        let main = Frame {
            x: 90.0,
            y: 470.0,
            w: 450.0,
            h: 300.0,
        };
        let xa = Axis::fit(c.x.iter().copied(), input.log_x, 0.03);
        let ya = Axis::fit(y_values(c), input.log_y, 0.05);
        let series = dataset_series(c, color(i), true, true);
        draw_plot(&mut p, &main, &series, &xa, &ya, None, &c.y_label, false);
        let y = 300.0;
        if !c.residual.is_empty() {
            let res = Frame {
                x: 90.0,
                y: 350.0,
                w: 450.0,
                h: 105.0,
            };
            let ra = Axis::fit(c.residual.iter().copied().chain([0.0]), false, 0.08);
            let rs = [Series {
                x: &c.residual_x,
                y: &c.residual,
                color: color(i),
                mark: Mark::Points,
                label: None,
            }];
            let label = if c.residual_weighted {
                "(y - fit) / sigma"
            } else {
                "y - fit"
            };
            draw_plot(&mut p, &res, &rs, &xa, &ra, Some(&c.x_label), label, true);
        }
        let rows = input
            .params
            .get(i)
            .map(|ps| param_rows(ps))
            .unwrap_or_default();
        pages.push(p);
        if !rows.is_empty() {
            let at = y;
            let last = pages.len() - 1;
            pages[last].text(50.0, at, 10.0, Font::Bold, "Parameters");
            flow_mono(&mut pages, at - 16.0, &rows, 8.0);
        }
    }

    // The full report text.
    if !input.report_text.trim().is_empty() {
        let mut p = Page::new(&font);
        p.text(50.0, PAGE_H - 55.0, 13.0, Font::Bold, "Fit report");
        pages.push(p);
        let lines: Vec<String> = input.report_text.lines().map(str::to_string).collect();
        flow_mono(&mut pages, PAGE_H - 80.0, &lines, 7.5);
    }

    // Page numbers.
    let n = pages.len();
    for (i, p) in pages.iter_mut().enumerate() {
        p.text_centered(
            PAGE_W / 2.0,
            25.0,
            7.5,
            Font::Regular,
            &format!("{} / {n}", i + 1),
        );
    }
    let font = font.borrow();
    assemble(&pages, &input.title, &font)
}

/// `s` as a PDF text string: literal when WinAnsi-safe, else UTF-16BE hex.
fn pdf_string(s: &str) -> String {
    if winansi(s) {
        format!("({})", pdf_text(s))
    } else {
        let mut hex = String::from("<FEFF");
        for u in s.encode_utf16() {
            let _ = write!(hex, "{u:04X}");
        }
        hex.push('>');
        hex
    }
}

fn stream_object(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// ToUnicode CMap so text in the PDF can be searched and copied.
fn to_unicode_cmap(glyphs: &BTreeMap<u16, (u32, String)>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = glyphs.iter().filter(|(g, _)| **g != 0).collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (gid, (_, text)) in chunk {
            let utf16: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(s, "<{gid:04X}> <{utf16}>");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

/// Serialise pages into a PDF 1.7 file (OpenType font programs need 1.6+) with
/// a correct cross-reference table.
fn assemble(pages: &[Page], title: &str, font: &Embedded) -> Vec<u8> {
    // 1 catalog, 2 pages, 3 Courier, 4 Type0 font, 5 CID font, 6 descriptor,
    // 7 font program, 8 ToUnicode, 9 info, then (page, contents) pairs.
    let first_page = 10;
    let page_ids: Vec<usize> = (0..pages.len()).map(|i| first_page + 2 * i).collect();
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = page_ids.iter().map(|id| format!("{id} 0 R")).collect();
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    );

    // The subset: only glyphs the pages used. The tag makes the subset's name
    // unique, as the PDF spec asks for subset fonts.
    let program = subsetter::subset(NOTO_SANS_JP, 0, &font.remap).unwrap_or_default();
    let hash = program.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let tag: String = (0..6)
        .map(|i| char::from(b'A' + ((hash >> (i * 5)) % 26) as u8))
        .collect();
    let base = format!("{tag}+{NOTO_NAME}");
    objects.push(
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 8 0 R >>")
            .into_bytes(),
    );
    let widths: Vec<String> = (0..font.remap.num_gids())
        .map(|g| font.glyphs.get(&g).map_or(1000, |(w, _)| *w).to_string())
        .collect();
    objects.push(
        format!(
            "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /{base} \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor 6 0 R /DW 1000 /W [0 [{}]] >>",
            widths.join(" ")
        )
        .into_bytes(),
    );
    let f = &font.face;
    let sc = font.scale();
    let b = f.global_bounding_box();
    let bbox = [b.x_min, b.y_min, b.x_max, b.y_max].map(|v| (f64::from(v) * sc).round() as i64);
    let cap = f.capital_height().unwrap_or(f.ascender());
    objects.push(
        format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [{} {} {} {}] /ItalicAngle 0 \
             /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile3 7 0 R >>",
            bbox[0],
            bbox[1],
            bbox[2],
            bbox[3],
            (f64::from(f.ascender()) * sc).round(),
            (f64::from(f.descender()) * sc).round(),
            (f64::from(cap) * sc).round(),
        )
        .into_bytes(),
    );
    objects.push(stream_object("/Subtype /OpenType", &program));
    objects.push(stream_object("", to_unicode_cmap(&font.glyphs).as_bytes()));
    objects.push(format!("<< /Title {} /Producer (fit_it) >>", pdf_string(title)).into_bytes());

    for (i, p) in pages.iter().enumerate() {
        let content_id = page_ids[i] + 1;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        objects.push(stream_object("", p.ops.as_bytes()));
    }

    let mut out: Vec<u8> = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 9 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{dataset_curves, export_set};
    use crate::model::testing::lookup;

    #[test]
    fn japanese_text_is_embedded_and_searchable() {
        let font: FontRef = Rc::new(RefCell::new(Embedded::new()));
        let mut page = Page::new(&font);
        page.text(50.0, 700.0, 12.0, Font::Regular, "濃度 50µM");
        page.text(50.0, 680.0, 8.0, Font::Mono, "plain mono");
        assert!(
            page.ops.contains("/F1 8 Tf"),
            "ASCII mono text stays in Courier"
        );
        let f = font.borrow();
        // .notdef plus 7 distinct characters (濃 度 space 5 0 µ M).
        assert_eq!(f.remap.num_gids(), 8);
        assert!(f.glyphs.values().any(|(w, t)| t == "濃" && *w == 1000));
        let cmap = to_unicode_cmap(&f.glyphs);
        assert!(
            cmap.contains("<6FC3>"),
            "ToUnicode maps back to 濃 (U+6FC3)"
        );
        let subset = subsetter::subset(NOTO_SANS_JP, 0, &f.remap).unwrap();
        assert!(subset.len() < 50_000, "{} bytes", subset.len());
        assert_eq!(pdf_string("fit"), "(fit)");
        assert_eq!(pdf_string("濃"), "<FEFF6FC3>");
    }

    #[test]
    fn text_is_winansi_escaped() {
        assert_eq!(pdf_text("a(b)\\"), "a\\(b\\)\\\\");
        assert_eq!(pdf_text("χ² ±"), "chi\\262 \\261");
        assert_eq!(pdf_text("—日"), "\\227?");
    }

    #[test]
    fn ticks_are_round_numbers() {
        let a = Axis {
            lo: -0.3,
            hi: 7.2,
            log: false,
        };
        let t: Vec<String> = a.ticks().into_iter().map(|t| t.1).collect();
        assert_eq!(t, ["0", "2", "4", "6"]);
        let l = Axis {
            lo: -3.2,
            hi: 0.5,
            log: true,
        };
        assert_eq!(l.ticks().first().unwrap().1, "1e-3");
    }

    #[test]
    fn report_is_a_well_formed_pdf() {
        let p = crate::export::tests::two_dataset_project();
        let set = export_set(&p, 0);
        let curves = dataset_curves(&p, &lookup, &set, 200, false).unwrap();
        let params = set.iter().map(|&i| p.datasets[i].params.clone()).collect();
        let pdf = render(&ReportInput {
            title: "fit_it report".into(),
            info: vec!["Global fit".into()],
            curves: &curves,
            colors: set.clone(),
            params,
            report_text: "[[Fit Statistics]]\n    chi-square = 1\n",
            log_x: false,
            log_y: false,
        });
        let text = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        // Overview + 2 dataset pages + report page.
        assert!(text.contains("/Count 4"));
        // Every xref offset points at the start of its object. Byte offsets: the
        // binary header comment makes a lossy string longer than the file.
        let tail = std::str::from_utf8(&pdf[pdf.len() - 40..]).unwrap();
        let xref_at: usize = tail
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let table = std::str::from_utf8(&pdf[xref_at..]).unwrap();
        let mut count = 0;
        for (i, line) in table
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .enumerate()
        {
            let off: usize = line[..10].parse().unwrap();
            assert!(
                pdf[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()),
                "object {}",
                i + 1
            );
            count += 1;
        }
        // Catalog, pages, Courier, 5 embedded-font objects, info + (page, contents) per page.
        assert_eq!(count, 9 + 2 * 4);
        // A subset, not the whole 4.5 MB font.
        assert!(pdf.len() < 600_000, "{} bytes", pdf.len());
    }
}
