//! UI language (English / 日本語) and the font Japanese text needs.
//!
//! Translations sit next to their English text at each call site —
//! `t("Datasets", "データセット")` — so they cannot drift out of sync with a
//! separate table. Fit reports stay in English: their lmfit layout is a de-facto
//! standard that people paste into notebooks and papers.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    En,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ja];

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ja => "日本語",
        }
    }
}

static JAPANESE: AtomicBool = AtomicBool::new(false);

pub fn set_lang(lang: Lang) {
    JAPANESE.store(lang == Lang::Ja, Ordering::Relaxed);
}

/// Pick the text for the current UI language. Works for `&str` and `String`
/// (e.g. two `format!`s).
pub fn t<T>(en: T, ja: T) -> T {
    if JAPANESE.load(Ordering::Relaxed) {
        ja
    } else {
        en
    }
}

/// Add the bundled Noto Sans JP after egui's own fonts, so Japanese UI text
/// and Japanese file names render (egui's fonts have no CJK glyphs). The same
/// font is embedded in PDF reports.
pub fn install_cjk_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "noto_sans_jp".into(),
        std::sync::Arc::new(egui::FontData::from_static(crate::report_pdf::NOTO_SANS_JP)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto_sans_jp".into());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_language() {
        set_lang(Lang::Ja);
        assert_eq!(t("Fit", "フィット"), "フィット");
        assert_eq!(t(format!("{} a", 1), format!("{} あ", 1)), "1 あ");
        set_lang(Lang::En);
        assert_eq!(t("Fit", "フィット"), "Fit");
    }
}
