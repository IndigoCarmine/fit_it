//! Serialising pages and the embedded font into PDF objects.

use super::font::{Embedded, NOTO_NAME, NOTO_SANS_JP, pdf_text, winansi};
use super::page::{PAGE_H, PAGE_W, Page};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// `s` as a PDF text string: literal when WinAnsi-safe, else UTF-16BE hex.
pub(super) fn pdf_string(s: &str) -> String {
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
pub(super) fn to_unicode_cmap(glyphs: &BTreeMap<u16, (u32, String)>) -> String {
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
pub(super) fn assemble(pages: &[Page], title: &str, font: &Embedded) -> Vec<u8> {
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
