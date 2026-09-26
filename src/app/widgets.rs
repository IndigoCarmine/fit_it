//! Small reusable widgets and formatting helpers.

use egui::{TextEdit, Ui};
use std::hash::Hash;

/// Compact number display: fixed notation for moderate magnitudes, scientific otherwise.
pub fn fmt_num(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let a = v.abs();
    if a == 0.0 || (1e-3..1e6).contains(&a) {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.to_string() }
    } else {
        // 1.23000e-7 -> 1.23e-7: short enough to survive narrow fields.
        let s = format!("{v:.5e}");
        match s.split_once('e') {
            Some((m, e)) if m.contains('.') => {
                format!("{}e{e}", m.trim_end_matches('0').trim_end_matches('.'))
            }
            _ => s,
        }
    }
}

fn parse_num(s: &str, empty: f64) -> Option<f64> {
    let s = s.trim();
    match s.to_ascii_lowercase().as_str() {
        "" => Some(empty),
        "inf" | "+inf" | "∞" => Some(f64::INFINITY),
        "-inf" | "-∞" => Some(f64::NEG_INFINITY),
        _ => s.parse().ok(),
    }
}

/// A text field editing a number. While focused it keeps the user's text; an
/// empty field means `empty` (e.g. -inf for a lower bound). Returns true on change.
pub fn num_field(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    value: &mut f64,
    width: f32,
    empty: f64,
    hint: &str,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let shown = |v: f64| {
        if v == empty && v.is_infinite() {
            String::new()
        } else {
            fmt_num(v)
        }
    };
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| shown(*value));
    let valid = parse_num(&text, empty).is_some();
    let mut edit = TextEdit::singleline(&mut text)
        .id(id.with("edit"))
        .desired_width(width)
        .hint_text(hint);
    if !valid {
        edit = edit.text_color(ui.visuals().error_fg_color);
    }
    let resp = ui.add(edit);
    let mut changed = false;
    if resp.has_focus() {
        if let Some(v) = parse_num(&text, empty)
            && v.to_bits() != value.to_bits()
        {
            *value = v;
            changed = true;
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    changed
}

/// A text field whose edits are only reported when committed (Enter or focus loss).
pub fn text_commit(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    current: &str,
    width: f32,
) -> Option<String> {
    let id = ui.make_persistent_id(id_salt);
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_string());
    let resp = ui.add(
        TextEdit::singleline(&mut text)
            .id(id.with("edit"))
            .desired_width(width),
    );
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        None
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
        (resp.lost_focus() && text.trim() != current).then(|| text.trim().to_string())
    }
}

/// Distinct colors for datasets (Okabe–Ito, readable in light and dark themes).
pub fn dataset_color(i: usize) -> egui::Color32 {
    let (r, g, b) = crate::export::PALETTE[i % crate::export::PALETTE.len()];
    egui::Color32::from_rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_compactly() {
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(0.0), "0");
        assert_eq!(fmt_num(-2.0), "-2");
        assert_eq!(fmt_num(1.23e-7), "1.23e-7");
        assert_eq!(fmt_num(5e-6), "5e-6");
        assert_eq!(fmt_num(f64::INFINITY), "inf");
    }

    #[test]
    fn parses_bounds() {
        assert_eq!(parse_num("", f64::NEG_INFINITY), Some(f64::NEG_INFINITY));
        assert_eq!(parse_num("-inf", 0.0), Some(f64::NEG_INFINITY));
        assert_eq!(parse_num(" 2e3 ", 0.0), Some(2000.0));
        assert_eq!(parse_num("abc", 0.0), None);
    }
}
