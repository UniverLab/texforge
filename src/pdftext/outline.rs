//! PDF outline (bookmark tree): entries, section numbers, page breaks.

use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::quality::{dict_text, resolve_dict};
use super::{PdfOutlineEntry, PdfPageBreak};

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
    let mut level_counters: Vec<usize> = vec![0; 1];

    if let Ok(first) = outlines_dict.get(b"First") {
        walk_outline_items(doc, first, 0, &pages_map, &mut entries, &mut level_counters);
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
    level_counters: &mut Vec<usize>,
) {
    while level_counters.len() <= level {
        level_counters.push(0);
    }
    level_counters[level] += 1;

    let Some(item_dict) = resolve_dict(doc, Some(first_obj)) else {
        return;
    };

    let title = dict_text(item_dict, b"Title").unwrap_or_default();
    let page = resolve_outline_page(doc, item_dict, pages_map);

    if let Some(page_num) = page {
        entries.push(PdfOutlineEntry {
            title,
            page: page_num,
            level,
        });
    }

    if let Ok(first_child) = item_dict.get(b"First") {
        walk_outline_items(
            doc,
            first_child,
            level + 1,
            pages_map,
            entries,
            level_counters,
        );
    }

    if let Ok(next) = item_dict.get(b"Next") {
        walk_outline_items(doc, next, level, pages_map, entries, level_counters);
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
        _ => None,
    }
}

fn resolve_named_dest(doc: &Document, name: &str) -> Option<ObjectId> {
    let catalog = doc.catalog().ok()?;
    let names_obj = catalog.get(b"Names").ok()?;
    let names_dict = resolve_dict(doc, Some(names_obj))?;
    let dests_obj = names_dict.get(b"Dests").ok()?;
    let dests_dict = resolve_dict(doc, Some(dests_obj))?;

    walk_named_dest_tree(doc, dests_dict, name)
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
    if let Some(dest_dict) = resolve_dict(doc, Some(chunk[1])) {
        let dest_arr = dest_dict.get(b"D").ok()?;
        let dest_items = resolve_array(doc, Some(dest_arr))?;
        let first = dest_items.first()?;
        let Object::Reference(page_id) = first else {
            return None;
        };
        return Some(*page_id);
    }
    let dest_items = resolve_array(doc, Some(chunk[1]))?;
    let first = dest_items.first()?;
    let Object::Reference(page_id) = first else {
        return None;
    };
    Some(*page_id)
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
    let mut entry_idx = 0;

    for page_num in 1..=num_pages {
        // A page is attributed to the FIRST section that opens it, never the
        // last — the rule `4947308` established for the text-matching path and
        // that this one must honour too. Several sections routinely start on
        // one page: in the capabilities example, page 2 opens sections 1, 1.1,
        // 1.2, 2, 2.1, 2.2 and 2.3, and the answer is 1. Later entries on the
        // same page are still consumed, just not reported.
        let mut opened_here = false;
        while entry_idx < numbered.len() && numbered[entry_idx].0 == page_num {
            if !opened_here {
                let (_, num, title) = &numbered[entry_idx];
                current = Some((num.clone(), title.clone()));
                opened_here = true;
            }
            entry_idx += 1;
        }
        out.push(PdfPageBreak {
            page: page_num,
            section: current.as_ref().map(|(n, _)| n.clone()),
            title: current.as_ref().map(|(_, t)| t.clone()),
        });
    }
    out
}

fn compute_section_numbers(entries: &[PdfOutlineEntry]) -> Vec<(usize, String, String)> {
    let mut counters: Vec<usize> = Vec::new();
    let mut result = Vec::with_capacity(entries.len());

    for entry in entries {
        let level = entry.level;
        while counters.len() <= level {
            counters.push(0);
        }
        counters[level] += 1;
        counters.truncate(level + 1);

        let number: String = counters
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(".");
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
mod tests {
    use super::super::fixtures::{CAPABILITIES_PDF, PAGES_PDF};
    use super::*;

    #[test]
    fn capabilities_pdf_has_outline() {
        let outline = read_pdf_outline_from_bytes(CAPABILITIES_PDF).unwrap();
        assert!(outline.is_some(), "capabilities PDF must have an outline");
        let (entries, page_count) = outline.unwrap();
        assert!(!entries.is_empty(), "outline must have entries");
        assert!(page_count > 0, "page count must be positive");
        assert!(
            entries.iter().any(|e| e.page > 0),
            "outline entries must resolve to pages"
        );
    }

    #[test]
    fn pages_ligatures_fixture_has_no_outline() {
        let outline = read_pdf_outline_from_bytes(PAGES_PDF).unwrap();
        assert!(
            outline.is_none(),
            "pages-ligatures fixture should not have an outline"
        );
    }

    #[test]
    fn page_breaks_from_outline_reports_the_first_section_on_a_page() {
        // Ground truth read from the capabilities PDF's own table of contents:
        // page 2 opens 1, 1.1, 1.2, 2, 2.1, 2.2 and 2.3. The answer is 1.
        // Before this was fixed the outline path reported 2.3 — the last
        // entry on the page — contradicting the rule the text-matching path
        // has followed since `4947308`.
        let e = |title: &str, page: usize, level: usize| PdfOutlineEntry {
            title: title.into(),
            page,
            level,
        };
        let entries = vec![
            e("Introduccion", 2, 0),
            e("Antecedentes", 2, 1),
            e("Objetivos", 2, 1),
            e("Diagramas", 2, 0),
            e("Mermaid", 2, 1),
            e("Graphviz", 2, 1),
            e("D2", 2, 1),
            e("Estilos", 5, 1),
        ];
        let breaks = page_breaks_from_outline(&entries, 5);

        assert_eq!(breaks[0].section, None, "page 1 is front matter");
        assert_eq!(breaks[1].section.as_deref(), Some("1"));
        assert_eq!(breaks[1].title.as_deref(), Some("Introduccion"));
        // Pages 3 and 4 open nothing: they carry the last opened section.
        assert_eq!(breaks[2].section.as_deref(), Some("1"));
        assert_eq!(breaks[3].section.as_deref(), Some("1"));
        assert_eq!(breaks[4].section.as_deref(), Some("2.4"));
    }

    #[test]
    fn page_breaks_from_outline_computes_section_numbers() {
        let entries = vec![
            PdfOutlineEntry {
                title: "Introduction".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Background".into(),
                page: 2,
                level: 1,
            },
            PdfOutlineEntry {
                title: "Methods".into(),
                page: 3,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 4);
        assert_eq!(breaks.len(), 4);
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[0].title.as_deref(), Some("Introduction"));
        assert_eq!(breaks[1].section.as_deref(), Some("1.1"));
        assert_eq!(breaks[1].title.as_deref(), Some("Background"));
        assert_eq!(breaks[2].section.as_deref(), Some("2"));
        assert_eq!(breaks[2].title.as_deref(), Some("Methods"));
        assert_eq!(breaks[3].section.as_deref(), Some("2"));
    }

    #[test]
    fn page_breaks_from_outline_handles_multiple_entries_on_same_page() {
        let entries = vec![
            PdfOutlineEntry {
                title: "First".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Second".into(),
                page: 1,
                level: 1,
            },
            PdfOutlineEntry {
                title: "Third".into(),
                page: 2,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 2);
        // The page is attributed to the FIRST section that opens it. This
        // asserted "1.1" — the last entry on the page — which described what
        // the code did rather than what the command promises ("which section
        // opens each page"), and contradicted the text-matching path.
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[0].title.as_deref(), Some("First"));
        assert_eq!(breaks[1].section.as_deref(), Some("2"));
        assert_eq!(breaks[1].title.as_deref(), Some("Third"));
    }

    #[test]
    fn page_breaks_from_outline_heading_less_page_in_middle_of_section() {
        let entries = vec![
            PdfOutlineEntry {
                title: "Introduction".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Methods".into(),
                page: 4,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 5);
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[1].section.as_deref(), Some("1"));
        assert_eq!(breaks[2].section.as_deref(), Some("1"));
        assert_eq!(breaks[3].section.as_deref(), Some("2"));
        assert_eq!(breaks[4].section.as_deref(), Some("2"));
    }

    #[test]
    fn page_breaks_from_outline_several_consecutive_heading_less_pages() {
        let entries = vec![
            PdfOutlineEntry {
                title: "First".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Second".into(),
                page: 5,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 6);
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[1].section.as_deref(), Some("1"));
        assert_eq!(breaks[2].section.as_deref(), Some("1"));
        assert_eq!(breaks[3].section.as_deref(), Some("1"));
        assert_eq!(breaks[4].section.as_deref(), Some("2"));
        assert_eq!(breaks[5].section.as_deref(), Some("2"));
    }

    #[test]
    fn page_breaks_from_outline_heading_less_page_after_subsection_ends() {
        let entries = vec![
            PdfOutlineEntry {
                title: "Introduction".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Background".into(),
                page: 2,
                level: 1,
            },
            PdfOutlineEntry {
                title: "Methods".into(),
                page: 4,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 5);
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[1].section.as_deref(), Some("1.1"));
        assert_eq!(breaks[2].section.as_deref(), Some("1.1"));
        assert_eq!(breaks[3].section.as_deref(), Some("2"));
        assert_eq!(breaks[4].section.as_deref(), Some("2"));
    }

    #[test]
    fn page_breaks_from_outline_pages_before_first_heading_are_unattributed() {
        let entries = vec![PdfOutlineEntry {
            title: "Introduction".into(),
            page: 3,
            level: 0,
        }];
        let breaks = page_breaks_from_outline(&entries, 4);
        assert_eq!(breaks[0].section.as_deref(), None);
        assert_eq!(breaks[1].section.as_deref(), None);
        assert_eq!(breaks[2].section.as_deref(), Some("1"));
        assert_eq!(breaks[3].section.as_deref(), Some("1"));
    }

    #[test]
    fn page_breaks_from_outline_page_with_heading_is_unaffected() {
        let entries = vec![
            PdfOutlineEntry {
                title: "First".into(),
                page: 1,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Second".into(),
                page: 2,
                level: 0,
            },
            PdfOutlineEntry {
                title: "Third".into(),
                page: 3,
                level: 0,
            },
        ];
        let breaks = page_breaks_from_outline(&entries, 3);
        assert_eq!(breaks[0].section.as_deref(), Some("1"));
        assert_eq!(breaks[1].section.as_deref(), Some("2"));
        assert_eq!(breaks[2].section.as_deref(), Some("3"));
    }
}
