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
