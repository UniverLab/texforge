//! PDF fixtures embedded into the test binary (see `super::tests`).

pub(super) const LIGATURES_PDF: &[u8] = include_bytes!("../../tests/fixtures/ligatures.pdf");
pub(super) const PAGES_PDF: &[u8] = include_bytes!("../../tests/fixtures/pages-ligatures.pdf");
pub(super) const MALFORMED_DATE_PDF: &[u8] =
    include_bytes!("../../tests/fixtures/malformed-date.pdf");

pub(super) const CAPABILITIES_PDF: &[u8] =
    include_bytes!("../../examples/texforge-capabilities/texforge-capabilities.pdf");

/// A one-page PDF whose only font has a `/ToUnicode` map for `A` but uses
/// both `A` and `B`, and carries no `/Encoding`: `pdf-extract` panics on it
/// (`missing unicode map and encoding`).
pub(super) fn missing_unicode_map_pdf() -> Vec<u8> {
    build_pdf("")
}

/// Same PDF but the font also carries `/Encoding /WinAnsiEncoding`: this one
/// extracts `AB` without panicking.
pub(super) fn missing_unicode_map_with_encoding_pdf() -> Vec<u8> {
    build_pdf(" /Encoding /WinAnsiEncoding")
}

/// Build the one-page fixture PDFs above. Objects 1-7 have the exact bodies
/// the extraction tests rely on; `extra_font_entries` is spliced into font
/// object 5 (empty for the panicking variant).
fn build_pdf(extra_font_entries: &str) -> Vec<u8> {
    let content = "BT /F1 24 Tf 20 40 Td (AB) Tj ET";
    let cmap = [
        "/CIDInit /ProcSet findresource begin",
        "12 dict begin",
        "begincmap",
        "/CMapName /Adobe-Identity-UCS def",
        "/CMapType 2 def",
        "1 begincodespacerange",
        "<00> <FF>",
        "endcodespacerange",
        "1 beginbfchar",
        "<41> <0041>",
        "endbfchar",
        "endcmap",
        "CMapName currentdict /CMap defineresource pop",
        "end",
        "end",
    ]
    .join("\n");
    let bodies: Vec<Vec<u8>> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".as_bytes().to_vec(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".as_bytes().to_vec(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .as_bytes()
            .to_vec(),
        stream_object(content.as_bytes()),
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /FixtureFont /FirstChar 65 /LastChar 66 \
             /Widths [600 600] /FontDescriptor 7 0 R /ToUnicode 6 0 R{extra_font_entries} >>"
        )
        .as_bytes()
        .to_vec(),
        stream_object(cmap.as_bytes()),
        "<< /Type /FontDescriptor /FontName /FixtureFont /Flags 4 /FontBBox [0 0 600 700] \
         /ItalicAngle 0 /Ascent 700 /Descent 0 /CapHeight 700 /StemV 80 >>"
            .as_bytes()
            .to_vec(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF").as_bytes(),
    );
    pdf
}

/// One indirect stream object body (without the `N 0 obj` / `endobj` lines).
fn stream_object(data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}
