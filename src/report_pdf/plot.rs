//! Vector plots: axes with round ticks, data points, curves and a legend.

use super::page::{BLACK, Font, GREY, LIGHT, Page, Rgb};
use std::fmt::Write as _;

/// Compact number for tick labels and tables.
pub(super) fn num(v: f64) -> String {
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
pub(super) struct Axis {
    lo: f64,
    hi: f64,
    log: bool,
}

impl Axis {
    pub(super) fn fit(values: impl Iterator<Item = f64>, log: bool, margin: f64) -> Axis {
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

pub(super) enum Mark {
    Points,
    Line {
        width: f64,
        dash: Option<(f64, f64)>,
    },
}

pub(super) struct Series<'a> {
    pub(super) x: &'a [f64],
    pub(super) y: &'a [f64],
    pub(super) color: Rgb,
    pub(super) mark: Mark,
    pub(super) label: Option<String>,
}

pub(super) struct Frame {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) w: f64,
    pub(super) h: f64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_plot(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
