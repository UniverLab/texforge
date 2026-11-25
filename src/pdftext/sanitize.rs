//! Rename `/Font` resource names inside form `XObjects` before extraction.
//!
//! `pdf-extract` caches decoded fonts by resource name for the whole page,
//! so a form `XObject` (an included figure) that calls its own font `F11`
//! while the page calls a different font `F11` decodes with whichever map
//! was built first. Giving every form font a name that is unique per form
//! object (`X<form object number>.<original name>`) removes the collision:
//! page-level fonts keep their names, each form gets its own.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};

/// How many form fonts [`rename_form_fonts`] renamed or gave up on.
pub(super) struct RenameReport {
    /// Forms whose fonts were renamed.
    pub renamed: usize,
    /// Forms with an unreadable content stream, left completely untouched.
    pub skipped: usize,
}

/// Give every font of every form `XObject` a per-form resource name.
///
/// Only streams with `/Subtype /Form` and their own `/Resources` holding a
/// `/Font` dictionary (direct, or an indirect reference to one) are
/// touched; everything else keeps its names. A form whose content cannot be
/// decompressed or decoded is left completely untouched and counted in
/// `skipped`. Never prints, never fails.
pub(super) fn rename_form_fonts(doc: &mut Document) -> RenameReport {
    let mut report = RenameReport {
        renamed: 0,
        skipped: 0,
    };
    // `objects` is a `BTreeMap`, so this visits each object once in
    // object-number order, including forms nested inside other forms.
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        rename_one_form(doc, id, &mut report);
    }
    report
}

/// A form that qualifies for renaming: where its `/Resources` lives and a
/// clone of its `/Font` entries (`None` means leave the object alone).
struct FormPlan {
    /// Indirect `/Resources` target, when the stream points at one.
    resources_ref: Option<ObjectId>,
    /// (`old name`, font object) pairs cloned from the `/Font` dictionary.
    fonts: Vec<(Vec<u8>, Object)>,
}

fn rename_one_form(doc: &mut Document, id: ObjectId, report: &mut RenameReport) {
    let Some(plan) = form_rename_plan(doc, id) else {
        return;
    };
    let Ok(mut content) = form_content(doc, id) else {
        report.skipped += 1;
        return;
    };
    let new_names = plan.new_names(id);
    for (old, new) in &new_names {
        rename_font_uses(&mut content, old, new);
    }
    let Ok(encoded) = content.encode() else {
        report.skipped += 1;
        return;
    };
    if store_plain_content(doc, id, encoded).is_err() {
        report.skipped += 1;
        return;
    }
    store_fresh_font_dict(doc, id, &plan, &new_names);
    report.renamed += 1;
}

impl FormPlan {
    /// (`old name`, `X<object number>.<old name>`) pairs, as bytes.
    fn new_names(&self, id: ObjectId) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.fonts
            .iter()
            .map(|(old, _)| {
                let mut new = format!("X{}.", id.0).into_bytes();
                new.extend_from_slice(old);
                (old.clone(), new)
            })
            .collect()
    }
}

/// Collect the rename plan for one object, or `None` to leave it alone.
///
/// Only a stream with `/Subtype /Form`, its own `/Resources`, and a
/// `/Font` dictionary inside those resources (direct or referenced)
/// qualifies. Anything else — pages included — uses its names by design.
fn form_rename_plan(doc: &Document, id: ObjectId) -> Option<FormPlan> {
    let stream = doc.get_object(id).ok()?.as_stream().ok()?;
    if stream.dict.get(b"Subtype").and_then(Object::as_name).ok()? != b"Form" {
        return None;
    }
    let resources = stream.dict.get(b"Resources").ok()?;
    let (resources_ref, resources_dict) = match resources {
        Object::Dictionary(dict) => (None, dict.clone()),
        Object::Reference(target) => (
            Some(*target),
            doc.get_object(*target).ok()?.as_dict().ok()?.clone(),
        ),
        _ => return None,
    };
    let fonts = match resources_dict.get(b"Font") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        Ok(Object::Reference(target)) => doc.get_object(*target).ok()?.as_dict().ok()?.clone(),
        _ => return None,
    };
    if fonts.is_empty() {
        return None;
    }
    let entries: Vec<(Vec<u8>, Object)> = fonts
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some(FormPlan {
        resources_ref,
        fonts: entries,
    })
}

/// Decompress and decode one stream's content (`Err` on unknown filter or
/// parse error; forms are usually `/FlateDecode`, which raw bytes hide).
fn form_content(doc: &Document, id: ObjectId) -> Result<Content<Vec<Operation>>, ()> {
    let stream = doc
        .get_object(id)
        .map_err(|_| ())?
        .as_stream()
        .map_err(|_| ())?;
    let raw = stream.decompressed_content().map_err(|_| ())?;
    Content::decode(&raw).map_err(|_| ())
}

/// Rewrite the first operand of every `Tf` that uses font `old` to `new`.
fn rename_font_uses(content: &mut Content<Vec<Operation>>, old: &[u8], new: &[u8]) {
    for operation in &mut content.operations {
        if operation.operator != "Tf" {
            continue;
        }
        let matches = matches!(
            operation.operands.first(),
            Some(Object::Name(name)) if name == old
        );
        if matches {
            operation.operands[0] = Object::Name(new.to_vec());
        }
    }
}

/// Re-store a form's content stream without its (now wrong) filters.
fn store_plain_content(doc: &mut Document, id: ObjectId, encoded: Vec<u8>) -> Result<(), ()> {
    let stream = doc
        .get_object_mut(id)
        .map_err(|_| ())?
        .as_stream_mut()
        .map_err(|_| ())?;
    stream.set_plain_content(encoded);
    Ok(())
}

/// Swap the form's `/Resources` `/Font` for a fresh direct dictionary that
/// holds the same font objects under the new per-form names.
///
/// When the stream borrows its `/Resources` from another object, the stream
/// gets its own direct copy: rewriting the shared object would rename the
/// fonts for every other borrower (another form, or even a page) that still
/// resolves the old names.
fn store_fresh_font_dict(
    doc: &mut Document,
    id: ObjectId,
    plan: &FormPlan,
    new_names: &[(Vec<u8>, Vec<u8>)],
) {
    let mut fresh = Dictionary::new();
    for ((_, new), (_, value)) in new_names.iter().zip(plan.fonts.iter()) {
        fresh.set(new.clone(), value.clone());
    }
    let font = Object::Dictionary(fresh);
    if let Some(target) = plan.resources_ref {
        if detach_resources(doc, id, target, font.clone()) {
            return;
        }
    }
    if let Ok(object) = doc.get_object_mut(id) {
        if let Ok(stream) = object.as_stream_mut() {
            if let Ok(Object::Dictionary(resources)) = stream.dict.get_mut(b"Resources") {
                resources.set("Font", font);
            }
        }
    }
}

/// Give the form stream `id` its own direct copy of the shared `/Resources`
/// object `target` with `/Font` swapped for `font`.
///
/// Returns false when `target` or the stream stopped being what the plan saw
/// (unreachable in practice: nothing else touches the document mid-rename).
fn detach_resources(doc: &mut Document, id: ObjectId, target: ObjectId, font: Object) -> bool {
    let detached = doc
        .get_object(target)
        .ok()
        .and_then(|object| object.as_dict().ok())
        .map(|resources| {
            let mut copy = resources.clone();
            copy.set("Font", font);
            Object::Dictionary(copy)
        });
    let Some(detached) = detached else {
        return false;
    };
    doc.get_object_mut(id)
        .ok()
        .and_then(|object| object.as_stream_mut().ok())
        .is_some_and(|stream| {
            stream.dict.set("Resources", detached);
            true
        })
}

/// The 14 standard PDF fonts (`pdf-extract`'s `is_core_font` list).
const CORE_FONTS: &[&[u8]] = &[
    b"Courier",
    b"Courier-Bold",
    b"Courier-BoldOblique",
    b"Courier-Oblique",
    b"Helvetica",
    b"Helvetica-Bold",
    b"Helvetica-BoldOblique",
    b"Helvetica-Oblique",
    b"Symbol",
    b"Times-Bold",
    b"Times-BoldItalic",
    b"Times-Italic",
    b"Times-Roman",
    b"ZapfDingbats",
];

/// Complete `/ToUnicode` `CMap`s on simple fonts that lack `/Encoding` so
/// unmapped byte codes decode to `U+FFFD` instead of panicking in `pdf-extract`.
///
/// Returns the number of font dictionaries patched.
pub(super) fn complete_unicode_maps(doc: &mut Document) -> usize {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut patched = 0;
    for id in ids {
        let Some(cmap_id) = font_cmap_to_patch(doc, id) else {
            continue;
        };
        if patch_cmap_stream(doc, cmap_id) {
            patched += 1;
        }
    }
    patched
}

fn font_cmap_to_patch(doc: &Document, id: ObjectId) -> Option<ObjectId> {
    let dict = doc.get_object(id).ok()?.as_dict().ok()?;
    if dict.has(b"Encoding") {
        return None;
    }
    let subtype = dict.get(b"Subtype").and_then(Object::as_name).ok()?;
    if matches!(subtype, b"Type0" | b"Type3") {
        return None;
    }
    if let Ok(base_font) = dict.get(b"BaseFont").and_then(Object::as_name) {
        if CORE_FONTS.contains(&base_font) {
            return None;
        }
    }
    match dict.get(b"ToUnicode").ok()? {
        Object::Reference(cmap_id) => Some(*cmap_id),
        _ => None,
    }
}

fn patch_cmap_stream(doc: &mut Document, id: ObjectId) -> bool {
    let Ok(raw) = doc
        .get_object(id)
        .and_then(Object::as_stream)
        .and_then(|stream| stream.decompressed_content())
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&raw);
    if text.contains("<00> <FFFD>") {
        return false;
    }
    let Some(pos) = [text.find("beginbfchar"), text.find("beginbfrange")]
        .into_iter()
        .flatten()
        .min()
        .or_else(|| text.find("endcmap"))
    else {
        return false;
    };
    let insert_at = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let filler = fffd_filler_cmap();
    let mut patched = String::with_capacity(text.len() + filler.len());
    patched.push_str(&text[..insert_at]);
    patched.push_str(&filler);
    patched.push_str(&text[insert_at..]);
    let Ok(stream) = doc.get_object_mut(id).and_then(Object::as_stream_mut) else {
        return false;
    };
    stream.set_plain_content(patched.into_bytes());
    true
}

fn fffd_filler_cmap() -> String {
    let mut out = String::new();
    for start in [0u16, 100, 200] {
        let end = (start + 99).min(255);
        let count = end - start + 1;
        out.push_str(&format!("{count} beginbfchar\n"));
        for code in start..=end {
            out.push_str(&format!("<{code:02X}> <FFFD>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::extract::extract_text_from_bytes;
    use super::super::fixtures::{missing_unicode_map_pdf, missing_unicode_map_with_encoding_pdf};
    use super::*;

    fn replace_cmap(doc: &mut Document, new_cmap: &str) {
        let cmap_id = doc
            .objects
            .values()
            .find_map(|obj| {
                let dict = obj.as_dict().ok()?;
                match dict.get(b"ToUnicode").ok()? {
                    Object::Reference(target) => Some(*target),
                    _ => None,
                }
            })
            .expect("fixture must have ToUnicode reference");
        let stream = doc
            .get_object_mut(cmap_id)
            .expect("cmap stream must exist")
            .as_stream_mut()
            .expect("cmap must be a stream");
        stream.set_plain_content(new_cmap.as_bytes().to_vec());
    }

    #[test]
    fn complete_unicode_maps_patches_once_and_skips_encoded() {
        let mut doc = Document::load_mem(&missing_unicode_map_pdf()).expect("fixture must load");
        assert_eq!(complete_unicode_maps(&mut doc), 1);
        assert_eq!(complete_unicode_maps(&mut doc), 0);

        let mut encoded = Document::load_mem(&missing_unicode_map_with_encoding_pdf())
            .expect("encoded fixture must load");
        assert_eq!(complete_unicode_maps(&mut encoded), 0);
    }

    #[test]
    fn complete_unicode_maps_skips_core_fonts() {
        let mut doc = Document::load_mem(&missing_unicode_map_pdf()).expect("fixture must load");
        for obj in doc.objects.values_mut() {
            if let Ok(dict) = obj.as_dict_mut() {
                if dict.has(b"ToUnicode") {
                    dict.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
                }
            }
        }
        assert_eq!(complete_unicode_maps(&mut doc), 0);
    }

    #[test]
    fn complete_unicode_maps_patches_before_endcmap_when_no_bf_blocks() {
        let mut doc = Document::load_mem(&missing_unicode_map_pdf()).expect("fixture must load");
        let empty_cmap = [
            "/CIDInit /ProcSet findresource begin",
            "12 dict begin",
            "begincmap",
            "1 begincodespacerange",
            "<00> <FF>",
            "endcodespacerange",
            "endcmap",
            "end",
            "end",
        ]
        .join("\n");
        replace_cmap(&mut doc, &empty_cmap);
        assert_eq!(complete_unicode_maps(&mut doc), 1);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("patched doc must save");
        let text = extract_text_from_bytes(&bytes).expect("must extract");
        assert!(
            text.contains("\u{FFFD}\u{FFFD}"),
            "expected two FFFDs, got {text:?}"
        );
    }

    #[test]
    fn complete_unicode_maps_preserves_bfrange_over_filler() {
        let mut doc = Document::load_mem(&missing_unicode_map_pdf()).expect("fixture must load");
        let range_cmap = [
            "/CIDInit /ProcSet findresource begin",
            "12 dict begin",
            "begincmap",
            "1 begincodespacerange",
            "<00> <FF>",
            "endcodespacerange",
            "1 beginbfrange",
            "<41> <41> <0041>",
            "endbfrange",
            "endcmap",
            "end",
            "end",
        ]
        .join("\n");
        replace_cmap(&mut doc, &range_cmap);
        assert_eq!(complete_unicode_maps(&mut doc), 1);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("patched doc must save");
        let text = extract_text_from_bytes(&bytes).expect("must extract");
        assert!(
            text.contains("A\u{FFFD}"),
            "expected A followed by FFFD, got {text:?}"
        );
        assert!(!text.contains('B'), "B must remain unmapped: {text:?}");
    }
}
