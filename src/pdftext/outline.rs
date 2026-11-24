//! PDF outline (bookmark tree): entries, section numbers, page breaks.

use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::quality::{dict_text, resolve_dict};
use super::{number_from_dest, DestNumber, PdfOutlineEntry, PdfPageBreak};

/// Read the PDF outline (bookmark tree) from a PDF file.
///
/// Returns `Ok(Some(entries))` when the PDF has an outline with at least one
/// entry that resolves to a page, `Ok(None)` when the PDF has no outline or
/// the outline is empty, and `Err` when the PDF cannot be parsed.
///
/// The outline lives inside compressed object streams in modern PDFs, so a
/// raw byte search will miss it; use the parser.
pub fn read_pdf_outline(path: &Path) -> Result<Option<(Vec<PdfOutlineEntry>, usize)>> {
    let doc =
        Document::load(path).with_context(|| format!("failed to open PDF {}", path.display()))?;
    read_pdf_outline_from_doc(&doc)
}

/// Read the PDF outline from an already-loaded document.
#[allow(dead_code)]
pub fn read_pdf_outline_from_bytes(data: &[u8]) -> Result<Option<(Vec<PdfOutlineEntry>, usize)>> {
    let doc = Document::load_mem(data).context("failed to parse PDF bytes")?;
    read_pdf_outline_from_doc(&doc)
}

fn read_pdf_outline_from_doc(doc: &Document) -> Result<Option<(Vec<PdfOutlineEntry>, usize)>> {
    let Ok(catalog) = doc.catalog() else {
        return Ok(None);
    };

    let Ok(outlines_obj) = catalog.get(b"Outlines") else {
        return Ok(None);
    };

    let Some(outlines_dict) = resolve_dict(doc, Some(outlines_obj)) else {
        return Ok(None);
    };

    let pages_map = doc.get_pages();
    let page_count = pages_map.len();
    let mut entries = Vec::new();

    if let Ok(first) = outlines_dict.get(b"First") {
        walk_outline_items(doc, first, 0, &pages_map, &mut entries);
    }

    if entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some((entries, page_count)))
    }
}

fn walk_outline_items(
    doc: &Document,
    first_obj: &Object,
    level: usize,
    pages_map: &std::collections::BTreeMap<u32, ObjectId>,
    entries: &mut Vec<PdfOutlineEntry>,
) {
    let Some(item_dict) = resolve_dict(doc, Some(first_obj)) else {
        return;
    };

    let title = dict_text(item_dict, b"Title").unwrap_or_default();
    let page = resolve_outline_page(doc, item_dict, pages_map);
    let dest = outline_dest_name(doc, item_dict);

    if let Some(page_num) = page {
        entries.push(PdfOutlineEntry {
            title,
            page: page_num,
            level,
            dest,
        });
    }

    if let Ok(first_child) = item_dict.get(b"First") {
        walk_outline_items(doc, first_child, level + 1, pages_map, entries);
    }

    if let Ok(next) = item_dict.get(b"Next") {
        walk_outline_items(doc, next, level, pages_map, entries);
    }
}

fn resolve_outline_page(
    doc: &Document,
    item: &Dictionary,
    pages_map: &std::collections::BTreeMap<u32, ObjectId>,
) -> Option<usize> {
    if let Ok(dest) = item.get(b"Dest") {
        if let Some(page_id) = extract_page_ref_from_dest(doc, dest) {
            return pages_map
                .iter()
                .find(|(_, &id)| id == page_id)
                .map(|(&n, _)| n as usize);
        }
    }

    if let Ok(action) = item.get(b"A") {
        if let Some(action_dict) = resolve_dict(doc, Some(action)) {
            if let Ok(dest) = action_dict.get(b"D") {
                if let Some(page_id) = extract_page_ref_from_dest(doc, dest) {
                    return pages_map
                        .iter()
                        .find(|(_, &id)| id == page_id)
                        .map(|(&n, _)| n as usize);
                }
            }
        }
    }

    None
}

/// The hyperref destination name behind an outline item's `/Dest` (or its
/// `/A` action's `/D`), when it is one: a PDF string or a PDF name object.
fn outline_dest_name(doc: &Document, item: &Dictionary) -> Option<String> {
    if let Ok(dest) = item.get(b"Dest") {
        if let Some(name) = dest_name(dest) {
            return Some(name);
        }
    }
    if let Ok(action) = item.get(b"A") {
        if let Some(action_dict) = resolve_dict(doc, Some(action)) {
            if let Ok(dest) = action_dict.get(b"D") {
                if let Some(name) = dest_name(dest) {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// The destination name when `dest` is a PDF string or a PDF name object.
fn dest_name(dest: &Object) -> Option<String> {
    match dest {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

fn extract_page_ref_from_dest(doc: &Document, dest: &Object) -> Option<ObjectId> {
    match dest {
        Object::Reference(id) => Some(*id),
        Object::Array(arr) => {
            if let Some(Object::Reference(page_id)) = arr.first() {
                Some(*page_id)
            } else {
                None
            }
        }
        Object::String(name, _) => {
            let name_str = String::from_utf8_lossy(name);
            resolve_named_dest(doc, &name_str)
        }
        Object::Name(name) => {
            let name_str = String::from_utf8_lossy(name);
            resolve_named_dest(doc, &name_str)
        }
        _ => None,
    }
}

fn resolve_named_dest(doc: &Document, name: &str) -> Option<ObjectId> {
    let catalog = doc.catalog().ok()?;
    named_tree_page(doc, catalog, name).or_else(|| catalog_dests_page(doc, catalog, name))
}

/// The page behind `name` in the catalog's `/Names /Dests` name tree.
fn named_tree_page(doc: &Document, catalog: &Dictionary, name: &str) -> Option<ObjectId> {
    let names_dict = resolve_dict(doc, catalog.get(b"Names").ok())?;
    let dests_dict = resolve_dict(doc, names_dict.get(b"Dests").ok())?;
    walk_named_dest_tree(doc, dests_dict, name)
}

/// The page behind `name` in a catalog-level `/Dests` dictionary, which
/// maps names straight to dests.
fn catalog_dests_page(doc: &Document, catalog: &Dictionary, name: &str) -> Option<ObjectId> {
    let dests_dict = resolve_dict(doc, catalog.get(b"Dests").ok())?;
    let dest_value = dests_dict.get(name.as_bytes()).ok()?;
    dest_page_ref(doc, dest_value)
}

/// The page reference behind a named-destination value: either a dict
/// holding the dest array under `/D` or a direct dest array.
fn dest_page_ref(doc: &Document, value: &Object) -> Option<ObjectId> {
    if let Some(dest_dict) = resolve_dict(doc, Some(value)) {
        if let Ok(dest_arr) = dest_dict.get(b"D") {
            let items = resolve_array(doc, Some(dest_arr))?;
            if let Object::Reference(page_id) = items.first()? {
                return Some(*page_id);
            }
            return None;
        }
    }
    let items = resolve_array(doc, Some(value))?;
    if let Object::Reference(page_id) = items.first()? {
        Some(*page_id)
    } else {
        None
    }
}

/// Match one `[name value]` pair from a `/Names` array against the wanted
/// destination name. The value is either a dict holding the dest array under
/// `/D` or a direct dest array whose first element is the page reference.
fn match_dest_name_entry(doc: &Document, chunk: &[&Object], name: &str) -> Option<ObjectId> {
    if chunk.len() != 2 {
        return None;
    }
    let Object::String(entry_name, _) = chunk[0] else {
        return None;
    };
    if String::from_utf8_lossy(entry_name) != name {
        return None;
    }
    dest_page_ref(doc, chunk[1])
}

/// Recurse into the `/Kids` subtrees of a named-destination node.
fn search_kids(doc: &Document, dict: &Dictionary, name: &str) -> Option<ObjectId> {
    let kids_arr = dict.get(b"Kids").ok()?;
    let kids = resolve_array(doc, Some(kids_arr))?;
    for kid in kids {
        let Some(kid_dict) = resolve_dict(doc, Some(kid)) else {
            continue;
        };
        if let Some(result) = walk_named_dest_tree(doc, kid_dict, name) {
            return Some(result);
        }
    }
    None
}

fn walk_named_dest_tree(doc: &Document, dict: &Dictionary, name: &str) -> Option<ObjectId> {
    if let Ok(names_arr) = dict.get(b"Names") {
        let Some(entries) = resolve_array(doc, Some(names_arr)) else {
            return search_kids(doc, dict, name);
        };
        for chunk in entries.chunks(2) {
            if let Some(page_id) = match_dest_name_entry(doc, chunk, name) {
                return Some(page_id);
            }
        }
    }

    search_kids(doc, dict, name)
}

fn resolve_array<'a>(doc: &'a Document, obj: Option<&'a Object>) -> Option<Vec<&'a Object>> {
    match obj? {
        Object::Array(a) => Some(a.iter().collect()),
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Array(a) => Some(a.iter().collect()),
            _ => None,
        },
        _ => None,
    }
}

/// Build page breaks from a PDF outline.
///
/// Computes section numbers from the outline's nesting levels and maps each
/// page to the section that opens it. A page with no outline entry carries
/// forward the section that was open when the page began; pages before the
/// first outline entry belong to no section.
pub fn page_breaks_from_outline(
    entries: &[PdfOutlineEntry],
    num_pages: usize,
) -> Vec<PdfPageBreak> {
    let numbered = compute_section_numbers(entries);
    let mut out = Vec::with_capacity(num_pages);
    let mut current: Option<(String, String)> = None;
    let mut pending = numbered.iter().peekable();

    for page_num in 1..=num_pages {
        // A page is attributed to the FIRST section that opens it, never the
        // last — the rule `4947308` established for the text-matching path and
        // that this one must honour too. Several sections routinely start on
        // one page: in the capabilities example, page 4 opens sections 2,
        // 2.1 and 2.2, and the answer is 2. Later entries on the
        // same page are still consumed, just not reported.
        let mut opened_here = false;
        while let Some((page, num, title)) = pending.peek() {
            if *page != page_num {
                break;
            }
            if !opened_here {
                current = Some((num.clone(), title.clone()));
                opened_here = true;
            }
            pending.next();
        }
        out.push(PdfPageBreak {
            page: page_num,
            section: current.as_ref().map(|(n, _)| n.clone()),
            title: current.as_ref().map(|(_, t)| t.clone()),
        });
    }
    out
}

/// Section numbers for outline entries: a hyperref destination name carries
/// the number LaTeX printed, so it wins outright. An entry without a usable
/// name falls back to counting the bookmark tree exactly as before.
fn compute_section_numbers(entries: &[PdfOutlineEntry]) -> Vec<(usize, String, String)> {
    let mut counters: Vec<usize> = Vec::new();
    let mut result = Vec::with_capacity(entries.len());

    for entry in entries {
        let level = entry.level;
        let number = match entry.dest.as_deref().map(number_from_dest) {
            Some(DestNumber::Numbered(numbered)) => {
                // The printed number neither advances the fallback counters
                // nor leaves stale deeper levels behind for the next entry
                // that still counts.
                counters.truncate(level + 1);
                numbered
            }
            Some(DestNumber::Unnumbered) => {
                counters.truncate(level + 1);
                String::new()
            }
            _ => {
                while counters.len() <= level {
                    counters.push(0);
                }
                counters[level] += 1;
                counters.truncate(level + 1);
                counters
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            }
        };
        result.push((entry.page, number, entry.title.clone()));
    }
    result
}

/// Format page breaks for diff-friendly output: one line per page.
pub fn format_page_breaks(breaks: &[PdfPageBreak]) -> String {
    let mut lines = Vec::with_capacity(breaks.len());
    for b in breaks {
        match (&b.section, &b.title) {
            (Some(num), Some(title)) => {
                lines.push(format!("page={} section={} title={}", b.page, num, title));
            }
            _ => lines.push(format!("page={} section= title=", b.page)),
        }
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "outline_tests.rs"]
mod tests;
