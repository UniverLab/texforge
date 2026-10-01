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
    let mut pending = numbered.iter().peekable();

    for page_num in 1..=num_pages {
        // A page is attributed to the FIRST section that opens it, never the
        // last — the rule `4947308` established for the text-matching path and
        // that this one must honour too. Several sections routinely start on
        // one page: in the capabilities example, page 2 opens sections 1, 1.1,
        // 1.2, 2, 2.1, 2.2 and 2.3, and the answer is 1. Later entries on the
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

    /// The path entry point must agree with the bytes one on the real
    /// fixture: every destination form the file uses has to resolve to the
    /// same page through either door.
    #[test]
    fn read_pdf_outline_from_path_matches_bytes_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capabilities.pdf");
        std::fs::write(&path, CAPABILITIES_PDF).unwrap();
        let from_path = read_pdf_outline(&path).unwrap();
        let from_bytes = read_pdf_outline_from_bytes(CAPABILITIES_PDF).unwrap();
        assert_eq!(from_path, from_bytes);
        let (entries, page_count) = from_path.expect("capabilities PDF must have an outline");
        assert_eq!(page_count, 10, "fixture page count");
        assert_eq!(entries.len(), 21, "fixture outline entry count");
    }

    /// Exact pages, titles and nesting from the capabilities PDF's own
    /// bookmark tree: page resolution is the whole point of the outline
    /// path, so a wrong page (or a flattened level) must fail loudly.
    #[test]
    fn capabilities_outline_resolves_exact_pages_and_nesting() {
        let (entries, _) = read_pdf_outline_from_bytes(CAPABILITIES_PDF)
            .unwrap()
            .expect("capabilities PDF must have an outline");
        assert_eq!(
            entries[0],
            PdfOutlineEntry {
                title: "Introducción".into(),
                page: 2,
                level: 0,
            }
        );
        assert_eq!(
            entries[7],
            PdfOutlineEntry {
                title: "Estilos de Diagrama (style)".into(),
                page: 5,
                level: 1,
            }
        );
        assert_eq!(
            entries[20],
            PdfOutlineEntry {
                title: "Conclusión".into(),
                // The caption, the listing index and the labelled listing push
                // the last section onto the final page of the 10-page
                // capabilities document.
                page: 10,
                level: 0,
            }
        );
        assert!(
            entries.iter().any(|e| e.level > 0),
            "the outline must nest: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e.page == 9),
            "some entry must resolve past page 1: {entries:?}"
        );
    }

    /// A destination expressed as an explicit array.
    fn array_dest(page: lopdf::ObjectId) -> lopdf::Object {
        use lopdf::Object;
        Object::Array(vec![
            Object::Reference(page),
            Object::Name(b"XYZ".to_vec()),
            Object::Null,
            Object::Null,
            Object::Null,
        ])
    }

    /// A `GoTo` action pointing at a named destination.
    fn go_to_action(name: &str) -> lopdf::Object {
        use lopdf::{dictionary, Object};
        Object::Dictionary(dictionary! {
            "S" => "GoTo",
            "D" => Object::string_literal(name),
        })
    }

    /// One outline item: `Title`/`Parent` plus whatever keys the form under
    /// test needs (`Dest`, `A`, `First`, `Next`).
    fn outline_item(
        title: &str,
        parent: lopdf::ObjectId,
        extra: Vec<(&str, lopdf::Object)>,
    ) -> lopdf::Object {
        use lopdf::{dictionary, Object};
        let mut dict = dictionary! {
            "Title" => Object::string_literal(title),
            "Parent" => parent,
        };
        for (key, value) in extra {
            dict.set(key, value);
        }
        Object::Dictionary(dict)
    }

    /// The object ids the synthetic outline shares between its parts.
    struct DestForms {
        outlines_id: lopdf::ObjectId,
        item1_id: lopdf::ObjectId,
        item2_id: lopdf::ObjectId,
        item3_id: lopdf::ObjectId,
        item4_id: lopdf::ObjectId,
        child_id: lopdf::ObjectId,
        page1_id: lopdf::ObjectId,
        page2_id: lopdf::ObjectId,
    }

    /// The four outline items the reader must walk — an array dest, its child
    /// reached through `/First`, a direct page-reference dest and two
    /// `/A`-action items — plus the outline root.
    fn insert_outline_items(doc: &mut lopdf::Document, ids: &DestForms) {
        use lopdf::{dictionary, Object};
        let DestForms {
            outlines_id,
            item1_id,
            item2_id,
            item3_id,
            item4_id,
            child_id,
            page1_id,
            page2_id,
        } = *ids;
        doc.objects.insert(
            item1_id,
            outline_item(
                "Array Form",
                outlines_id,
                vec![
                    ("Dest", array_dest(page1_id)),
                    ("First", Object::Reference(child_id)),
                    ("Next", Object::Reference(item2_id)),
                ],
            ),
        );
        doc.objects.insert(
            child_id,
            outline_item(
                "Child Of Array",
                item1_id,
                vec![("Dest", array_dest(page2_id))],
            ),
        );
        doc.objects.insert(
            item2_id,
            outline_item(
                "Direct Form",
                outlines_id,
                vec![
                    ("Dest", Object::Reference(page2_id)),
                    ("Next", Object::Reference(item3_id)),
                ],
            ),
        );
        doc.objects.insert(
            item3_id,
            outline_item(
                "Named Dict Form",
                outlines_id,
                vec![
                    ("A", go_to_action("named-dict")),
                    ("Next", Object::Reference(item4_id)),
                ],
            ),
        );
        doc.objects.insert(
            item4_id,
            outline_item(
                "Named Array Form",
                outlines_id,
                vec![("A", go_to_action("named-array"))],
            ),
        );
        doc.objects.insert(
            outlines_id,
            Object::Dictionary(dictionary! {
                "First" => Object::Reference(item1_id),
                "Last" => Object::Reference(item4_id),
                "Count" => 5,
            }),
        );
    }

    /// The `/Names` tree behind the two action items: one destination stored
    /// as a dict holding `/D`, one as a direct dest array.
    fn insert_named_destinations(
        doc: &mut lopdf::Document,
        names_id: lopdf::ObjectId,
        dests_id: lopdf::ObjectId,
        dest_dict_id: lopdf::ObjectId,
        page1_id: lopdf::ObjectId,
        page2_id: lopdf::ObjectId,
    ) {
        use lopdf::{dictionary, Object};
        doc.objects.insert(
            dest_dict_id,
            Object::Dictionary(dictionary! {
                "D" => array_dest(page2_id),
            }),
        );
        doc.objects.insert(
            dests_id,
            Object::Dictionary(dictionary! {
                "Names" => vec![
                    Object::string_literal("named-dict"),
                    Object::Reference(dest_dict_id),
                    Object::string_literal("named-array"),
                    array_dest(page1_id),
                ],
            }),
        );
        doc.objects.insert(
            names_id,
            Object::Dictionary(dictionary! {
                "Dests" => Object::Reference(dests_id),
            }),
        );
    }

    /// A two-page document exercising every destination form the reader
    /// understands: an array dest, a direct page-reference dest, a child
    /// item reached through `/First`, and two named destinations behind
    /// `/A` actions — one stored as a dict holding `/D`, one as a direct
    /// dest array.
    fn doc_with_every_dest_form() -> lopdf::Document {
        use lopdf::{dictionary, Document, Object};

        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let page1_id = doc.new_object_id();
        let page2_id = doc.new_object_id();
        let outlines_id = doc.new_object_id();
        let item1_id = doc.new_object_id();
        let child_id = doc.new_object_id();
        let item2_id = doc.new_object_id();
        let item3_id = doc.new_object_id();
        let item4_id = doc.new_object_id();
        let names_id = doc.new_object_id();
        let dests_id = doc.new_object_id();
        let dest_dict_id = doc.new_object_id();

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page1_id), Object::Reference(page2_id)],
                "Count" => 2,
            }),
        );
        let page = Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
        });
        doc.objects.insert(page1_id, page.clone());
        doc.objects.insert(page2_id, page);

        insert_outline_items(
            &mut doc,
            &DestForms {
                outlines_id,
                item1_id,
                item2_id,
                item3_id,
                item4_id,
                child_id,
                page1_id,
                page2_id,
            },
        );
        insert_named_destinations(
            &mut doc,
            names_id,
            dests_id,
            dest_dict_id,
            page1_id,
            page2_id,
        );

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(pages_id),
            "Outlines" => Object::Reference(outlines_id),
            "Names" => Object::Reference(names_id),
        });
        doc.trailer.set("Root", catalog_id);

        doc
    }

    #[test]
    fn every_dest_form_resolves_to_its_own_page() {
        let doc = doc_with_every_dest_form();
        let (entries, page_count) = read_pdf_outline_from_doc(&doc)
            .unwrap()
            .expect("synthetic outline must resolve");
        assert_eq!(page_count, 2);
        let compact: Vec<(&str, usize, usize)> = entries
            .iter()
            .map(|e| (e.title.as_str(), e.page, e.level))
            .collect();
        assert_eq!(
            compact,
            vec![
                ("Array Form", 1, 0),
                ("Child Of Array", 2, 1),
                ("Direct Form", 2, 0),
                ("Named Dict Form", 2, 0),
                ("Named Array Form", 1, 0),
            ],
            "every destination form must land on its own page"
        );
    }
}
