//! `texforge pdf` — text extraction, info, page breaks, and fidelity check.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::commands::outline;
use crate::domain::project::Project;
use crate::linter::Severity;
use crate::pdftext::{self, PdfInfo};
use crate::utils::sanitize_filename;

/// Subcommands of `texforge pdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfAction {
    /// Print extracted text (`--raw` keeps ligature codepoints).
    Text { raw: bool },
    /// Pages, fonts (embedded?), metadata.
    Info,
    /// Diff-friendly page → section map.
    Pages,
    /// Source-to-PDF fidelity: missing significant words as warnings.
    Check,
}

/// Dispatch a `texforge pdf` action for the project in the current directory.
pub fn execute(action: PdfAction) -> Result<()> {
    let project = Project::load()?;
    execute_for_project(&project, action)
}

fn execute_for_project(project: &Project, action: PdfAction) -> Result<()> {
    let pdf_path = compiled_pdf_path(project);
    if !pdf_path.exists() {
        bail!(
            "No compiled PDF at {} — run `texforge build` first",
            pdf_path.display()
        );
    }

    match action {
        PdfAction::Text { raw } => cmd_text(&pdf_path, raw),
        PdfAction::Info => cmd_info(&pdf_path),
        PdfAction::Pages => cmd_pages(project, &pdf_path),
        PdfAction::Check => cmd_check(project, &pdf_path),
    }
}

fn compiled_pdf_path(project: &Project) -> PathBuf {
    project.root.join(format!(
        "{}.pdf",
        sanitize_filename(&project.config.document.title)
    ))
}

fn cmd_text(pdf_path: &Path, raw: bool) -> Result<()> {
    let extracted = pdftext::extract_text(pdf_path)?;
    let text = if raw {
        extracted
    } else {
        pdftext::normalize_pdf_text(&extracted)
    };
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn cmd_info(pdf_path: &Path) -> Result<()> {
    let info = pdftext::pdf_info(pdf_path)?;
    print_info(&info);
    Ok(())
}

fn print_info(info: &PdfInfo) {
    println!("pages: {}", info.pages);
    println!("fonts: {}", info.fonts.len());
    for font in &info.fonts {
        println!(
            "  name={} subtype={} embedded={}",
            font.name, font.subtype, font.embedded
        );
    }
    let m = &info.metadata;
    println!("metadata:");
    print_meta("title", m.title.as_deref());
    print_meta("author", m.author.as_deref());
    print_meta("subject", m.subject.as_deref());
    print_meta("keywords", m.keywords.as_deref());
    print_meta("creator", m.creator.as_deref());
    print_meta("producer", m.producer.as_deref());
    print_meta("creation_date", m.creation_date.as_deref());
    print_meta("mod_date", m.mod_date.as_deref());
}

fn print_meta(key: &str, value: Option<&str>) {
    match value {
        Some(v) => println!("  {key}={v}"),
        None => println!("  {key}="),
    }
}

fn cmd_pages(project: &Project, pdf_path: &Path) -> Result<()> {
    let page_texts = pdftext::extract_text_by_pages(pdf_path)?;
    let outline = outline::build_outline(
        &project.config.document.title,
        &project.root,
        &project.config.build.entry,
    );
    let sections: Vec<(String, String)> = outline
        .sections
        .iter()
        .map(|s| (s.number.clone(), s.title.clone()))
        .collect();
    let breaks = pdftext::page_breaks(&page_texts, &sections);
    println!("{}", pdftext::format_page_breaks(&breaks));
    Ok(())
}

fn cmd_check(project: &Project, pdf_path: &Path) -> Result<()> {
    let mut findings =
        pdftext::check_fidelity(&project.root, &project.config.build.entry, pdf_path)?;
    findings.extend(pdftext::check_quality(pdf_path)?);

    if findings.is_empty() {
        println!("  ◇ PDF check: fidelity, fonts, and metadata look good");
        return Ok(());
    }

    let warnings: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .collect();

    println!("WARNINGS ({})", warnings.len());
    for f in &warnings {
        println!("  WARNING [{}]", f.file);
        println!("    {}", f.message);
        if let Some(ref s) = f.suggestion {
            println!("    suggestion: {s}");
        }
        println!();
    }

    // Findings are warnings only — exit zero unless the caller
    // pipes through `check --deny-warnings`. Distinct words, not flood.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{BuildConfig, DocumentConfig, ProjectConfig};
    use std::fs;

    const LIGATURES: &[u8] = include_bytes!("../../tests/fixtures/ligatures.pdf");
    const PAGES: &[u8] = include_bytes!("../../tests/fixtures/pages-ligatures.pdf");

    fn project_with_pdf(
        pdf_bytes: &[u8],
        title: &str,
        main_tex: &str,
    ) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::write(
            root.join("project.toml"),
            format!(
                r#"[document]
title = "{title}"
author = "Test"
template = "general"

[build]
entry = "main.tex"
"#
            ),
        )
        .unwrap();
        fs::write(root.join("main.tex"), main_tex).unwrap();
        let pdf_name = format!("{}.pdf", sanitize_filename(title));
        fs::write(root.join(pdf_name), pdf_bytes).unwrap();
        let project = Project {
            root,
            config: ProjectConfig {
                document: DocumentConfig {
                    title: title.into(),
                    author: "Test".into(),
                    template: "general".into(),
                },
                build: BuildConfig {
                    entry: "main.tex".into(),
                    bibliography: None,
                    reproducible: None,
                },
                diagrams: None,
            },
        };
        (dir, project)
    }

    #[test]
    fn text_normalizes_ligatures_by_default() {
        let (_dir, project) = project_with_pdf(
            LIGATURES,
            "Lig Doc",
            r"\documentclass{article}\begin{document}Artificial\end{document}",
        );
        execute_for_project(&project, PdfAction::Text { raw: false }).unwrap();
    }

    #[test]
    fn text_raw_keeps_ligature_codepoints() {
        let (_dir, project) = project_with_pdf(
            LIGATURES,
            "Lig Doc",
            r"\documentclass{article}\begin{document}Artificial\end{document}",
        );
        let pdf = compiled_pdf_path(&project);
        let raw = pdftext::extract_text(&pdf).unwrap();
        assert!(raw.contains('\u{FB01}'));
        execute_for_project(&project, PdfAction::Text { raw: true }).unwrap();
    }

    #[test]
    fn info_and_pages_and_check_run() {
        let tex = r#"\documentclass{article}
\begin{document}
\section{Introduction}
Artificial Intelligence and MLflow workflows for local-first systems.
Deep Learning appears here with enough filler text to eventually force a page break if we add more.
\newpage
\section{Methods}
More Artificial Intelligence content on page two about workflows.
\end{document}
"#;
        let (_dir, project) = project_with_pdf(PAGES, "Pages Doc", tex);
        execute_for_project(&project, PdfAction::Info).unwrap();
        execute_for_project(&project, PdfAction::Pages).unwrap();
        execute_for_project(&project, PdfAction::Check).unwrap();
    }

    #[test]
    fn pages_attributes_macro_wrapped_heading_to_its_own_page() {
        // Same fixture PDF as `info_and_pages_and_check_run`, but the
        // heading opening page 2 is wrapped in `\textit`. Before titles
        // were resolved at the tokenizer, the raw markup title never
        // matched the plain page text, so page 2 stayed attributed to
        // section 1 instead of section 2.
        let tex = r#"\documentclass{article}
\begin{document}
\section{Introduction}
Artificial Intelligence and MLflow workflows for local-first systems.
Deep Learning appears here with enough filler text to eventually force a page break if we add more.
\newpage
\section{\textit{Methods}}
More Artificial Intelligence content on page two about workflows.
\end{document}
"#;
        let (_dir, project) = project_with_pdf(PAGES, "Pages Doc Macro", tex);
        let pdf_path = compiled_pdf_path(&project);
        let page_texts = pdftext::extract_text_by_pages(&pdf_path).unwrap();
        let outline = outline::build_outline(
            &project.config.document.title,
            &project.root,
            &project.config.build.entry,
        );
        let sections: Vec<(String, String)> = outline
            .sections
            .iter()
            .map(|s| (s.number.clone(), s.title.clone()))
            .collect();
        assert_eq!(
            sections[1].1, "Methods",
            "title should be resolved, not raw markup"
        );

        let breaks = pdftext::page_breaks(&page_texts, &sections);
        assert_eq!(
            pdftext::format_page_breaks(&breaks),
            "page=1 section=1 title=Introduction\npage=2 section=2 title=Methods"
        );
    }

    #[test]
    fn pages_does_not_let_an_unmatchable_heading_hide_a_later_section() {
        // TE5: page 2 opens with a macro-wrapped `\section` (an `\href`),
        // exactly the shape that survived the 2c16831 title-resolution fix
        // because that fix targeted `outline`, not this mapper. A
        // dot-leader subsection sits between "Introduction" and the `\href`
        // section — its resolved title never appears verbatim in the PDF
        // text (the PDF renders a literal row of dots; the resolved title
        // strips it to a space) — and previously that unmatched title
        // permanently blocked every section after it, including the one
        // that truly opens page 2.
        let tex = r#"\documentclass{article}
\begin{document}
\section{Introduction}
Artificial Intelligence and MLflow workflows for local-first systems.
Deep Learning appears here with enough filler text to eventually force a page break if we add more.
\subsection{AI Engineer en Accenture
\textcolor{lightgray}{\leaders\hbox{.}\hfill}
\textit{Julio 2026 -- Actual}}
\newpage
\section{\href{https://example.com}{Methods}}
More Artificial Intelligence content on page two about workflows.
\end{document}
"#;
        let (_dir, project) = project_with_pdf(PAGES, "Pages Doc Href", tex);
        let pdf_path = compiled_pdf_path(&project);
        let page_texts = pdftext::extract_text_by_pages(&pdf_path).unwrap();
        let outline = outline::build_outline(
            &project.config.document.title,
            &project.root,
            &project.config.build.entry,
        );
        let sections: Vec<(String, String)> = outline
            .sections
            .iter()
            .map(|s| (s.number.clone(), s.title.clone()))
            .collect();
        assert_eq!(
            sections[2].1, "Methods",
            "title should be resolved, not raw markup"
        );

        let breaks = pdftext::page_breaks(&page_texts, &sections);
        assert_eq!(
            pdftext::format_page_breaks(&breaks),
            "page=1 section=1 title=Introduction\npage=2 section=2 title=Methods"
        );
    }

    #[test]
    fn missing_pdf_is_helpful() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::write(
            root.join("project.toml"),
            r#"[document]
title = "No Pdf"
author = "Test"
template = "general"

[build]
entry = "main.tex"
"#,
        )
        .unwrap();
        fs::write(
            root.join("main.tex"),
            r"\documentclass{article}\begin{document}Hi\end{document}",
        )
        .unwrap();
        let project = Project {
            root,
            config: ProjectConfig {
                document: DocumentConfig {
                    title: "No Pdf".into(),
                    author: "Test".into(),
                    template: "general".into(),
                },
                build: BuildConfig {
                    entry: "main.tex".into(),
                    bibliography: None,
                    reproducible: None,
                },
                diagrams: None,
            },
        };
        let err = execute_for_project(&project, PdfAction::Info).unwrap_err();
        assert!(err.to_string().contains("texforge build"));
    }
}
