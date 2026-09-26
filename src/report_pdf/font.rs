//! The embedded Noto Sans JP subset, and WinAnsi text for the standard fonts.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Noto Sans JP Regular (SIL Open Font License 1.1; see resources/fonts/OFL.txt).
/// Also the app's UI font for Japanese.
pub const NOTO_SANS_JP: &[u8] = include_bytes!("../../resources/fonts/NotoSansJP-Regular.otf");
pub(super) const NOTO_NAME: &str = "NotoSansJP-Regular";

/// The embedded font as the report is written: glyphs are renumbered on first
/// use (the numbers go straight into the page content) and subset at the end.
pub(super) struct Embedded {
    pub(super) face: ttf_parser::Face<'static>,
    pub(super) remap: subsetter::GlyphRemapper,
    /// New glyph id -> (advance in 1/1000 em, text it stands for).
    pub(super) glyphs: BTreeMap<u16, (u32, String)>,
}

impl Embedded {
    pub(super) fn new() -> Self {
        Self {
            face: ttf_parser::Face::parse(NOTO_SANS_JP, 0).expect("bundled font parses"),
            remap: subsetter::GlyphRemapper::new(),
            glyphs: BTreeMap::new(),
        }
    }

    pub(super) fn scale(&self) -> f64 {
        1000.0 / f64::from(self.face.units_per_em())
    }

    /// Hex string of new glyph ids for `s` (Identity-H), and its width in 1/1000 em.
    pub(super) fn encode(&mut self, s: &str) -> (String, f64) {
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

    pub(super) fn width(&self, s: &str) -> f64 {
        s.chars()
            .map(|ch| {
                let g = self.face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                f64::from(self.face.glyph_hor_advance(g).unwrap_or(0)) * self.scale()
            })
            .sum()
    }
}

/// Can `s` be printed in a standard (WinAnsi) font without substitutions?
pub(super) fn winansi(s: &str) -> bool {
    s.chars()
        .all(|c| (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c))
}

/// WinAnsi-encode `s` as a PDF string literal body.
pub(super) fn pdf_text(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_winansi_escaped() {
        assert_eq!(pdf_text("a(b)\\"), "a\\(b\\)\\\\");
        assert_eq!(pdf_text("χ² ±"), "chi\\262 \\261");
        assert_eq!(pdf_text("—日"), "\\227?");
    }
}
