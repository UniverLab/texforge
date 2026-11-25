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

/// Assemble numbered indirect objects (1-based) into one PDF 1.4 file.
fn assemble_pdf(bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// A `/ToUnicode` `CMap` stream mapping byte `0x41` (`A`) to `unicode`
/// (a four-hex-digit UCS-2 code point such as `0050` for `P`).
fn clash_cmap(unicode: &str) -> Vec<u8> {
    [
        "/CIDInit /ProcSet findresource begin",
        "12 dict begin",
        "begincmap",
        "/CMapName /Adobe-Identity-UCS def",
        "/CMapType 2 def",
        "1 begincodespacerange",
        "<00> <FF>",
        "endcodespacerange",
        "1 beginbfchar",
        &format!("<41> <{unicode}>"),
        "endbfchar",
        "endcmap",
        "CMapName currentdict /CMap defineresource pop",
        "end",
        "end",
    ]
    .join("\n")
    .into_bytes()
}

/// A Type1 font object body whose `ToUnicode` map is object `cmap` and whose
/// descriptor is object `desc`.
fn clash_font(base_name: &str, desc: u32, cmap: u32) -> Vec<u8> {
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /{base_name} /FirstChar 65 \
         /LastChar 65 /Widths [600] /FontDescriptor {desc} 0 R /ToUnicode {cmap} 0 R >>"
    )
    .into_bytes()
}

/// A minimal font descriptor body for `font_name`.
fn clash_descriptor(font_name: &str) -> Vec<u8> {
    format!(
        "<< /Type /FontDescriptor /FontName /{font_name} /Flags 4 \
         /FontBBox [0 0 600 700] /ItalicAngle 0 /Ascent 700 /Descent 0 \
         /CapHeight 700 /StemV 80 >>"
    )
    .into_bytes()
}

/// A `/Subtype /Form` stream body with its own `/Resources` mapping `/F1`
/// to font object `font_id` and the given content.
fn clash_form_stream(font_id: u32, content: &str) -> Vec<u8> {
    let mut body = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] \
         /Resources << /Font << /F1 {font_id} 0 R >> >> /Length {} >>\nstream\n",
        content.len()
    )
    .into_bytes();
    body.extend_from_slice(content.as_bytes());
    body.extend_from_slice(b"\nendstream");
    body
}

/// A one-page PDF whose page font `F1` decodes `A` as `P` while the form
/// `Fm1`'s own `F1` decodes `A` as `Q`. When `form_first` is false the page
/// draws its `A` before invoking the form, otherwise after.
pub(super) fn font_name_clash_pdf(form_first: bool) -> Vec<u8> {
    let page_content = if form_first {
        "/Fm1 Do BT /F1 24 Tf 20 70 Td (A) Tj ET"
    } else {
        "BT /F1 24 Tf 20 70 Td (A) Tj ET /Fm1 Do"
    };
    let form_content = "BT /F1 24 Tf 0 0 Td (A) Tj ET";
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".as_bytes().to_vec(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
            .as_bytes()
            .to_vec(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R >> >> >>"
            .as_bytes()
            .to_vec(),
        stream_object(page_content.as_bytes()),
        clash_font("ClashP", 6, 7),
        clash_descriptor("ClashP"),
        stream_object(&clash_cmap("0050")),
        clash_form_stream(9, form_content),
        clash_font("ClashQ", 10, 11),
        clash_descriptor("ClashQ"),
        stream_object(&clash_cmap("0051")),
    ])
}

/// A one-page PDF whose page font `F1` decodes `A` as `P` while two forms
/// with their own `F1` decode it as `Q` (`M1`) and `R` (`M2`).
pub(super) fn font_name_clash_two_forms_pdf() -> Vec<u8> {
    let page_content = "BT /F1 24 Tf 20 70 Td (A) Tj ET /M1 Do /M2 Do";
    let form_content = "BT /F1 24 Tf 0 0 Td (A) Tj ET";
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".as_bytes().to_vec(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
            .as_bytes()
            .to_vec(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> /XObject << /M1 8 0 R /M2 12 0 R >> >> >>"
            .as_bytes()
            .to_vec(),
        stream_object(page_content.as_bytes()),
        clash_font("ClashP", 6, 7),
        clash_descriptor("ClashP"),
        stream_object(&clash_cmap("0050")),
        clash_form_stream(9, form_content),
        clash_font("ClashQ", 10, 11),
        clash_descriptor("ClashQ"),
        stream_object(&clash_cmap("0051")),
        clash_form_stream(13, form_content),
        clash_font("ClashR", 14, 15),
        clash_descriptor("ClashR"),
        stream_object(&clash_cmap("0052")),
    ])
}

/// A `/Subtype /Form` stream body borrowing a shared `/Resources` object.
fn shared_resources_form_stream(resources_id: u32, content: &str) -> Vec<u8> {
    let mut body = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] \
         /Resources {resources_id} 0 R /Length {} >>\nstream\n",
        content.len()
    )
    .into_bytes();
    body.extend_from_slice(content.as_bytes());
    body.extend_from_slice(b"\nendstream");
    body
}

/// A one-page PDF whose two forms borrow the SAME `/Resources` object mapping
/// `F1` to the `Q` font, while the page maps `F1` to `P`.
pub(super) fn font_name_clash_shared_resources_pdf() -> Vec<u8> {
    let page_content = "BT /F1 24 Tf 20 70 Td (A) Tj ET /M1 Do /M2 Do";
    let form_content = "BT /F1 24 Tf 0 0 Td (A) Tj ET";
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".as_bytes().to_vec(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
            .as_bytes()
            .to_vec(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> /XObject << /M1 8 0 R /M2 9 0 R >> >> >>"
            .as_bytes()
            .to_vec(),
        stream_object(page_content.as_bytes()),
        clash_font("ClashP", 6, 7),
        clash_descriptor("ClashP"),
        stream_object(&clash_cmap("0050")),
        shared_resources_form_stream(13, form_content),
        shared_resources_form_stream(13, form_content),
        clash_font("ClashQ", 11, 12),
        clash_descriptor("ClashQ"),
        stream_object(&clash_cmap("0051")),
        "<< /Font << /F1 10 0 R >> >>".as_bytes().to_vec(),
    ])
}

/// The page-then-form clash fixture built through `lopdf` objects instead
/// of raw bytes, with the form stream `/FlateDecode`-compressed.
pub(super) fn font_name_clash_compressed_pdf() -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.4");
    let add_font = |doc: &mut Document, base: &str, unicode: &str| {
        let descriptor = doc.add_object(dictionary! {
            "Type" => Object::Name(b"FontDescriptor".to_vec()),
            "FontName" => Object::Name(base.as_bytes().to_vec()),
            "Flags" => Object::Integer(4),
            "FontBBox" => Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(600),
                Object::Integer(700),
            ]),
            "ItalicAngle" => Object::Integer(0),
            "Ascent" => Object::Integer(700),
            "Descent" => Object::Integer(0),
            "CapHeight" => Object::Integer(700),
            "StemV" => Object::Integer(80),
        });
        let cmap = doc.add_object(Stream::new(dictionary! {}, clash_cmap(unicode)));
        doc.add_object(dictionary! {
            "Type" => Object::Name(b"Font".to_vec()),
            "Subtype" => Object::Name(b"Type1".to_vec()),
            "BaseFont" => Object::Name(base.as_bytes().to_vec()),
            "FirstChar" => Object::Integer(65),
            "LastChar" => Object::Integer(65),
            "Widths" => Object::Array(vec![Object::Integer(600)]),
            "FontDescriptor" => Object::Reference(descriptor),
            "ToUnicode" => Object::Reference(cmap),
        })
    };
    // Long comment padding so `Stream::compress` saves enough bytes to bother
    // with `/FlateDecode` (it skips streams that barely shrink). Comments
    // decode to no operations, so extraction is unaffected.
    let padded_form_content = format!(
        "BT /F1 24 Tf 0 0 Td (A) Tj ET{}",
        "% padding to make FlateDecode worthwhile 0123456789\n".repeat(20)
    );
    let font_p = add_font(&mut doc, "ClashP", "0050");
    let font_q = add_font(&mut doc, "ClashQ", "0051");
    let mut form = Stream::new(
        dictionary! {
            "Type" => Object::Name(b"XObject".to_vec()),
            "Subtype" => Object::Name(b"Form".to_vec()),
            "BBox" => Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(200),
                Object::Integer(100),
            ]),
            "Resources" => Object::Dictionary(dictionary! {
                "Font" => Object::Dictionary(dictionary! {
                    "F1" => Object::Reference(font_q),
                }),
            }),
        },
        padded_form_content.into_bytes(),
    );
    form.compress().expect("fixture form stream must compress");
    let form_id = doc.add_object(form);
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        "BT /F1 24 Tf 20 70 Td (A) Tj ET /Fm1 Do"
            .as_bytes()
            .to_vec(),
    ));
    let pages_id = doc.add_object(dictionary! {
        "Type" => Object::Name(b"Pages".to_vec()),
        "Kids" => Object::Array(vec![]),
        "Count" => Object::Integer(1),
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => Object::Name(b"Page".to_vec()),
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(200),
            Object::Integer(100),
        ]),
        "Contents" => Object::Reference(content_id),
        "Resources" => Object::Dictionary(dictionary! {
            "Font" => Object::Dictionary(dictionary! {
                "F1" => Object::Reference(font_p),
            }),
            "XObject" => Object::Dictionary(dictionary! {
                "Fm1" => Object::Reference(form_id),
            }),
        }),
    });
    if let Ok(Object::Array(kids)) = doc
        .get_object_mut(pages_id)
        .expect("pages object must exist")
        .as_dict_mut()
        .expect("pages object must be a dict")
        .get_mut(b"Kids")
    {
        kids.push(Object::Reference(page_id));
    }
    let catalog_id = doc.add_object(dictionary! {
        "Type" => Object::Name(b"Catalog".to_vec()),
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("fixture PDF must save");
    bytes
}
