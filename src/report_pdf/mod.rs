//! PDF fit reports, written directly (no PDF library): A4 pages with vector
//! plots of every fitted dataset, parameter tables and the lmfit-style report.
//!
//! Text uses the bundled Noto Sans JP (SIL Open Font License), embedded as a
//! subset holding only the glyphs the report uses, so Japanese file names and
//! symbols print correctly while the file stays small. The monospaced report
//! and tables use the standard Courier font (not embedded) when they are plain
//! WinAnsi text, which keeps their columns aligned.
//!
//! `font` holds the embedded font and text encoding, `page` a page's drawing
//! operators, `plot` axes and plots, and `writer` the PDF file structure; this
//! module lays out the report.

mod font;
mod page;
mod plot;
mod writer;

pub use font::NOTO_SANS_JP;

use crate::export::{DatasetCurves, PALETTE};
use crate::fit::Param;
use font::Embedded;
use page::{Font, FontRef, PAGE_H, PAGE_W, Page, Rgb, rgb};
use plot::{Axis, Frame, Mark, Series, draw_plot, num};
use std::cell::RefCell;
use std::rc::Rc;

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

impl ReportInput<'_> {
    /// Colour of curve `i`.
    fn color(&self, i: usize) -> Rgb {
        rgb(PALETTE[self.colors.get(i).copied().unwrap_or(i) % PALETTE.len()])
    }
}

/// Data points, components (dashed) and the fit of one dataset, all labelled.
fn dataset_series(c: &DatasetCurves, color: Rgb) -> Vec<Series<'_>> {
    let mut s = Vec::new();
    let tag = &c.tag;
    s.push(Series {
        x: &c.x,
        y: &c.y,
        color,
        mark: Mark::Points,
        label: Some(format!("{tag} data")),
    });
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
            label: Some(name.clone()),
        });
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
            label: Some(format!("{tag} fit")),
        });
    }
    s
}

fn y_values(c: &DatasetCurves) -> impl Iterator<Item = f64> + '_ {
    c.y.iter().chain(&c.grid_model).copied()
}

/// `num(v)`, or `inf` / `-inf` for an open bound.
fn bound(v: f64, open: &str) -> String {
    if v.is_finite() { num(v) } else { open.into() }
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
            format!("[{}, {}]", bound(p.min, "-inf"), bound(p.max, "inf"))
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

/// `line` split into chunks of at most `max_chars`; continuation chunks get a
/// four-space hanging indent (within the limit).
fn wrap_hanging(line: &str, max_chars: usize) -> Vec<String> {
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
    chunks
}

/// Mono text lines onto pages starting at `y`, adding pages as needed.
fn flow_mono(pages: &mut Vec<Page>, mut y: f64, lines: &[String], size: f64) {
    let step = size * 1.3;
    let max_chars = ((PAGE_W - 100.0) / (size * 0.6)) as usize;
    for line in lines {
        for chunk in wrap_hanging(line, max_chars) {
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

/// Page 1: title, info, overview of all datasets.
fn overview_page(font: &FontRef, input: &ReportInput) -> Page {
    let mut p = Page::new(font);
    p.text(50.0, PAGE_H - 60.0, 18.0, Font::Bold, &input.title);
    let mut y = PAGE_H - 82.0;
    for line in &input.info {
        p.text(50.0, y, 9.0, Font::Regular, line);
        y -= 13.0;
    }
    if input.curves.is_empty() {
        return p;
    }
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
        let col = input.color(i);
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
        p.color(input.color(i), true);
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
    p
}

/// Fit and residual plots of curve `i`; its parameter table goes below.
fn dataset_page(font: &FontRef, input: &ReportInput, i: usize) -> Page {
    let c = &input.curves[i];
    let mut p = Page::new(font);
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
    let series = dataset_series(c, input.color(i));
    draw_plot(&mut p, &main, &series, &xa, &ya, None, &c.y_label, false);
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
            color: input.color(i),
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
    p
}

/// Top of the parameter table on a dataset page.
const PARAMS_Y: f64 = 300.0;

pub fn render(input: &ReportInput) -> Vec<u8> {
    let font: FontRef = Rc::new(RefCell::new(Embedded::new()));
    let mut pages = vec![overview_page(&font, input)];

    // One page per dataset: fit, residuals, parameters.
    for i in 0..input.curves.len() {
        pages.push(dataset_page(&font, input, i));
        let rows = input
            .params
            .get(i)
            .map(|ps| param_rows(ps))
            .unwrap_or_default();
        if !rows.is_empty() {
            let last = pages.len() - 1;
            pages[last].text(50.0, PARAMS_Y, 10.0, Font::Bold, "Parameters");
            flow_mono(&mut pages, PARAMS_Y - 16.0, &rows, 8.0);
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
    writer::assemble(&pages, &input.title, &font)
}

#[cfg(test)]
mod tests {
    use super::writer::{pdf_string, to_unicode_cmap};
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
    fn long_lines_wrap_with_a_hanging_indent() {
        assert_eq!(wrap_hanging("", 10), [""]);
        assert_eq!(wrap_hanging("abcdefghij", 10), ["abcdefghij"]);
        assert_eq!(
            wrap_hanging("abcdefghijklmnop", 10),
            ["abcdefghij", "    klmnop"]
        );
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
