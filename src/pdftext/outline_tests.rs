
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
    // page 4 opens 2, 2.1 and 2.2. The answer is 2.
    // Before this was fixed the outline path reported 2.2 — the last
    // entry on the page — contradicting the rule the text-matching path
    // has followed since `4947308`.
    let e = |title: &str, page: usize, level: usize| PdfOutlineEntry {
        title: title.into(),
        page,
        level,
        dest: None,
    };
    let entries = vec![
        e("Qué es texforge", 3, 0),
        e("El flujo de trabajo", 4, 0),
        e("Inspeccionar el resultado", 4, 1),
        e("Los comandos", 4, 1),
        e("Diagramas", 6, 0),
        e("Mermaid", 6, 1),
        e("Graphviz", 6, 1),
        e("D2", 7, 1),
    ];
    let breaks = page_breaks_from_outline(&entries, 7);

    assert_eq!(breaks[0].section, None, "page 1 is front matter");
    assert_eq!(breaks[1].section, None, "page 2 is front matter");
    assert_eq!(breaks[2].section.as_deref(), Some("1"));
    assert_eq!(breaks[2].title.as_deref(), Some("Qué es texforge"));
    // Page 5 opens nothing: it carries the last opened section.
    assert_eq!(breaks[3].section.as_deref(), Some("2"));
    assert_eq!(breaks[4].section.as_deref(), Some("2"));
    assert_eq!(breaks[5].section.as_deref(), Some("3"));
    assert_eq!(breaks[6].section.as_deref(), Some("3.3"));
}

#[test]
fn page_breaks_from_outline_computes_section_numbers() {
    let entries = vec![
        PdfOutlineEntry {
            title: "Introduction".into(),
            page: 1,
            level: 0,
            dest: None,
        },
        PdfOutlineEntry {
            title: "Background".into(),
            page: 2,
            level: 1,
            dest: None,
        },
        PdfOutlineEntry {
            title: "Methods".into(),
            page: 3,
            level: 0,
            dest: None,
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
            dest: None,
        },
        PdfOutlineEntry {
            title: "Second".into(),
            page: 1,
            level: 1,
            dest: None,
        },
        PdfOutlineEntry {
            title: "Third".into(),
            page: 2,
            level: 0,
            dest: None,
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
            dest: None,
        },
        PdfOutlineEntry {
            title: "Methods".into(),
            page: 4,
            level: 0,
            dest: None,
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
            dest: None,
        },
        PdfOutlineEntry {
            title: "Second".into(),
            page: 5,
            level: 0,
            dest: None,
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
            dest: None,
        },
        PdfOutlineEntry {
            title: "Background".into(),
            page: 2,
            level: 1,
            dest: None,
        },
        PdfOutlineEntry {
            title: "Methods".into(),
            page: 4,
            level: 0,
            dest: None,
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
        dest: None,
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
            dest: None,
        },
        PdfOutlineEntry {
            title: "Second".into(),
            page: 2,
            level: 0,
            dest: None,
        },
        PdfOutlineEntry {
            title: "Third".into(),
            page: 3,
            level: 0,
            dest: None,
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
    assert_eq!(page_count, 19, "fixture page count");
    assert_eq!(entries.len(), 28, "fixture outline entry count");
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
            title: "Qué es texforge".into(),
            page: 3,
            level: 0,
            dest: Some("section.1".into()),
        }
    );
    assert_eq!(
        entries[8],
        PdfOutlineEntry {
            title: "D2".into(),
            page: 9,
            level: 1,
            dest: Some("subsection.3.3".into()),
        }
    );
    assert_eq!(
        entries[27],
        PdfOutlineEntry {
            title: "Conclusión".into(),
            page: 19,
            level: 0,
            dest: Some("section.9".into()),
        }
    );
    assert!(
        entries.iter().any(|e| e.level > 0),
        "the outline must nest: {entries:?}"
    );
    assert!(
        entries.iter().any(|e| e.page > 9),
        "some entry must resolve past page 9: {entries:?}"
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
    go_to_action_dest(lopdf::Object::string_literal(name))
}

/// A `GoTo` action pointing at an explicit destination object: a string
/// name (what hyperref writes) or a name object.
fn go_to_action_dest(dest: lopdf::Object) -> lopdf::Object {
    use lopdf::{dictionary, Object};
    Object::Dictionary(dictionary! {
        "S" => "GoTo",
        "D" => dest,
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

/// The hyperref destination names behind the five-entry outline: two
/// unnumbered/numbered chapters, a subsection, and two appendices.
const HYPERREF_NAMES: [&str; 5] = [
    "chapter*.1",
    "chapter.1",
    "section.1.1",
    "appendix.Alph1",
    "section.Alph1.2",
];

/// How the five outline entries below point at their pages.
#[derive(Clone, Copy, Debug)]
enum FiveEntryForm {
    /// `/A` `GoTo` actions with string `/D` names (what hyperref writes).
    StringNames,
    /// `/A` `GoTo` actions with name-object `/D` names.
    NameObjects,
    /// Plain page-array `/Dest`s: no names, the old counter path.
    PlainArrays,
}

/// Five pages with a five-entry outline (`Resumen`, two numbered
/// sections, two appendices) resolved through a `/Names /Dests` tree,
/// unless `form` carries plain page destinations instead.
fn doc_with_five_entries(form: FiveEntryForm) -> lopdf::Document {
    use lopdf::{dictionary, Document, Object};

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_ids: Vec<lopdf::ObjectId> = (0..5).map(|_| doc.new_object_id()).collect();
    let outlines_id = doc.new_object_id();
    let item_ids: Vec<lopdf::ObjectId> = (0..5).map(|_| doc.new_object_id()).collect();

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
            "Count" => 5,
        }),
    );
    for page_id in &page_ids {
        doc.objects.insert(
            *page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => Object::Reference(pages_id),
            }),
        );
    }

    let titles = [
        "Resumen",
        "Introduccion",
        "Antecedentes",
        "Anexo A",
        "Datos",
    ];
    for (idx, item_id) in item_ids.iter().enumerate() {
        let target = match form {
            FiveEntryForm::StringNames => go_to_action(HYPERREF_NAMES[idx]),
            FiveEntryForm::NameObjects => {
                go_to_action_dest(Object::Name(HYPERREF_NAMES[idx].as_bytes().to_vec()))
            }
            FiveEntryForm::PlainArrays => array_dest(page_ids[idx]),
        };
        let key = match form {
            FiveEntryForm::PlainArrays => "Dest",
            _ => "A",
        };
        let mut extra = vec![(key, target)];
        match idx {
            0 => extra.push(("Next", Object::Reference(item_ids[1]))),
            1 => {
                extra.push(("First", Object::Reference(item_ids[2])));
                extra.push(("Next", Object::Reference(item_ids[3])));
            }
            3 => extra.push(("First", Object::Reference(item_ids[4]))),
            _ => {}
        }
        let parent = match idx {
            2 => item_ids[1],
            4 => item_ids[3],
            _ => outlines_id,
        };
        doc.objects
            .insert(*item_id, outline_item(titles[idx], parent, extra));
    }
    doc.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "First" => Object::Reference(item_ids[0]),
            "Last" => Object::Reference(item_ids[3]),
            "Count" => 5,
        }),
    );

    let mut catalog = dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
        "Outlines" => Object::Reference(outlines_id),
    };
    if !matches!(form, FiveEntryForm::PlainArrays) {
        let names_id = doc.new_object_id();
        let dests_id = doc.new_object_id();
        let mut names_vec = Vec::new();
        for (idx, name) in HYPERREF_NAMES.iter().enumerate() {
            names_vec.push(Object::string_literal(*name));
            names_vec.push(array_dest(page_ids[idx]));
        }
        doc.objects.insert(
            dests_id,
            Object::Dictionary(dictionary! {
                "Names" => names_vec,
            }),
        );
        doc.objects.insert(
            names_id,
            Object::Dictionary(dictionary! {
                "Dests" => Object::Reference(dests_id),
            }),
        );
        catalog.set("Names", Object::Reference(names_id));
    }
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc
}

#[test]
fn named_destinations_yield_printed_numbers() {
    for form in [FiveEntryForm::StringNames, FiveEntryForm::NameObjects] {
        let doc = doc_with_five_entries(form);
        let (entries, page_count) = read_pdf_outline_from_doc(&doc)
            .unwrap()
            .expect("hyperref outline must resolve");
        assert_eq!(page_count, 5);
        let dests: Vec<Option<&str>> = entries.iter().map(|e| e.dest.as_deref()).collect();
        assert_eq!(
            dests,
            vec![
                Some("chapter*.1"),
                Some("chapter.1"),
                Some("section.1.1"),
                Some("appendix.Alph1"),
                Some("section.Alph1.2"),
            ]
        );
        let breaks = page_breaks_from_outline(&entries, 5);
        let sections: Vec<Option<&str>> = breaks.iter().map(|b| b.section.as_deref()).collect();
        assert_eq!(
            sections,
            vec![Some(""), Some("1"), Some("1.1"), Some("A"), Some("A.2")],
            "printed numbers win over bookmark counting ({form:?})"
        );
    }
}

#[test]
fn plain_destinations_keep_counter_numbers() {
    let doc = doc_with_five_entries(FiveEntryForm::PlainArrays);
    let (entries, page_count) = read_pdf_outline_from_doc(&doc)
        .unwrap()
        .expect("plain outline must resolve");
    assert_eq!(page_count, 5);
    assert!(
        entries.iter().all(|e| e.dest.is_none()),
        "plain dests carry no name: {entries:?}"
    );
    let breaks = page_breaks_from_outline(&entries, 5);
    let sections: Vec<Option<&str>> = breaks.iter().map(|b| b.section.as_deref()).collect();
    assert_eq!(
        sections,
        vec![Some("1"), Some("2"), Some("2.1"), Some("3"), Some("3.1")],
        "without names the bookmark tree is counted as before"
    );
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
