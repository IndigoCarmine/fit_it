//! One page's content stream: text, lines, shapes.

use super::font::{Embedded, pdf_text, winansi};
use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

pub(super) type FontRef = Rc<RefCell<Embedded>>;

pub(super) const PAGE_W: f64 = 595.0;
pub(super) const PAGE_H: f64 = 842.0;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Font {
    Regular,
    /// The regular face drawn with a thin outline (Noto Sans JP is bundled in
    /// one weight only).
    Bold,
    Mono,
}

pub(super) type Rgb = (f64, f64, f64);

pub(super) fn rgb((r, g, b): (u8, u8, u8)) -> Rgb {
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

pub(super) const BLACK: Rgb = (0.0, 0.0, 0.0);
pub(super) const GREY: Rgb = (0.55, 0.55, 0.55);
pub(super) const LIGHT: Rgb = (0.88, 0.88, 0.88);
pub(super) struct Page {
    pub(super) ops: String,
    pub(super) font: FontRef,
}

impl Page {
    pub(super) fn new(font: &FontRef) -> Self {
        Self {
            ops: String::new(),
            font: font.clone(),
        }
    }

    /// Courier for plain monospaced text; the embedded font for everything else.
    fn use_courier(font: Font, s: &str) -> bool {
        font == Font::Mono && winansi(s)
    }

    pub(super) fn width(&self, s: &str, size: f64, font: Font) -> f64 {
        if Self::use_courier(font, s) {
            s.chars().count() as f64 * size * 0.6
        } else {
            self.font.borrow().width(s) * size / 1000.0
        }
    }

    pub(super) fn color(&mut self, (r, g, b): Rgb, stroke: bool) {
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

    pub(super) fn text(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        self.show(&format!("1 0 0 1 {x:.2} {y:.2}"), size, font, s);
    }

    pub(super) fn text_centered(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        let w = self.width(s, size, font);
        self.text(x - w / 2.0, y, size, font, s);
    }

    pub(super) fn text_right(&mut self, x: f64, y: f64, size: f64, font: Font, s: &str) {
        let w = self.width(s, size, font);
        self.text(x - w, y, size, font, s);
    }

    /// Text rotated 90° counter-clockwise, centred on (x, y).
    pub(super) fn text_vertical(&mut self, x: f64, y: f64, size: f64, s: &str) {
        let w = self.width(s, size, Font::Regular);
        self.show(
            &format!("0 1 -1 0 {x:.2} {:.2}", y - w / 2.0),
            size,
            Font::Regular,
            s,
        );
    }

    pub(super) fn line_style(&mut self, width: f64, dash: Option<(f64, f64)>) {
        let _ = writeln!(self.ops, "{width:.2} w");
        match dash {
            Some((a, b)) => {
                let _ = writeln!(self.ops, "[{a} {b}] 0 d");
            }
            None => self.ops.push_str("[] 0 d\n"),
        }
    }

    pub(super) fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        let _ = writeln!(self.ops, "{x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S");
    }

    pub(super) fn polyline(&mut self, pts: &[(f64, f64)]) {
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

    pub(super) fn circle(&mut self, x: f64, y: f64, r: f64) {
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

    pub(super) fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let _ = writeln!(self.ops, "{x:.2} {y:.2} {w:.2} {h:.2} re S");
    }

    pub(super) fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb) {
        self.color(c, false);
        let _ = writeln!(self.ops, "{x:.2} {y:.2} {w:.2} {h:.2} re f");
    }
}
