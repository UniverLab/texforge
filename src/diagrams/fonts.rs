//! Font database for SVG → PDF/PNG conversion.
//!
//! Split out of `diagrams/mod.rs` (which past the file-size limit) with no
//! behaviour change: `svg_to_pdf` and `svg_to_png` in the parent module are
//! the only consumers, through [`shared_fontdb`] and [`shared_svg2pdf_fontdb`].
//!
//! Two `fontdb` versions live in the dependency graph — 0.24 through our own
//! `resvg` 0.48, and 0.23 through `svg2pdf` 0.13's bundled `usvg` 0.45 — and
//! they are distinct, incompatible types. The loader is therefore authored
//! once as [`define_fontdb_loader!`] and expanded into one populated database
//! per side of the SVG → PDF/PNG boundary; the SVG string stays the only thing
//! that crosses that boundary (see [`svg_to_pdf`](super::svg_to_pdf)).

use std::sync::Arc;

/// Generate a private module that builds and caches one populated font
/// database of type `$db`: system fonts, the Windows font directory under
/// WSL, fallback directories, and the generic CSS families.
///
/// The body is shared so both `fontdb` versions get byte-for-byte the same
/// configuration despite being unrelated types.
macro_rules! define_fontdb_loader {
    ($scope:ident, $db:ty) => {
        mod $scope {
            use std::sync::{Arc, OnceLock};

            /// Shared font database — building it scans system font directories
            /// (very slow on WSL, where /mnt/c/Windows/Fonts goes through the
            /// 9P filesystem), so it is built once and reused for every diagram.
            pub(super) fn shared() -> Arc<$db> {
                static FONTDB: OnceLock<Arc<$db>> = OnceLock::new();
                FONTDB.get_or_init(|| Arc::new(build())).clone()
            }

            /// Build a font database with system fonts and platform-specific fallbacks.
            fn build() -> $db {
                let mut db = <$db>::new();
                load_system_and_platform_fonts(&mut db);
                load_fallback_font_directories(&mut db);
                configure_font_families(&mut db);

                db
            }

            /// Load system fonts and platform-specific fonts (Windows/WSL).
            fn load_system_and_platform_fonts(db: &mut $db) {
                db.load_system_fonts();

                // On WSL / Windows, also load the Windows font directory
                let win_fonts = std::path::Path::new("/mnt/c/Windows/Fonts");
                if win_fonts.is_dir() {
                    db.load_fonts_dir(win_fonts);
                }
            }

            /// Load fallback font directories if no fonts were found.
            fn load_fallback_font_directories(db: &mut $db) {
                // If the DB still has no fonts at all, try common directories explicitly.
                if db.is_empty() {
                    for dir in ["/usr/share/fonts", "/usr/local/share/fonts"] {
                        let p = std::path::Path::new(dir);
                        if p.is_dir() {
                            db.load_fonts_dir(p);
                        }
                    }
                }
            }

            /// Configure font families based on available fonts.
            fn configure_font_families(db: &mut $db) {
                // Collect the set of available family names once (avoids borrow conflicts).
                let available: std::collections::HashSet<String> = db
                    .faces()
                    .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
                    .collect();

                // Map generic CSS families to the first concrete font we find in the DB.
                configure_sans_serif_family(db, &available);
                configure_serif_family(db, &available);
                configure_monospace_family(db, &available);
            }

            /// Configure sans-serif font family.
            ///
            /// D2 diagrams reference embedded font-family names that never resolve directly,
            /// so their text relies entirely on this sans-serif fallback. If none of the
            /// preferred fonts exist, fall back to any available family so text never
            /// silently disappears on minimal systems.
            fn configure_sans_serif_family(
                db: &mut $db,
                available: &std::collections::HashSet<String>,
            ) {
                let sans = ["Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"];
                if let Some(f) = sans.iter().find(|n| available.contains(**n)) {
                    db.set_sans_serif_family(*f);
                } else if let Some(any) = available.iter().next() {
                    db.set_sans_serif_family(any.clone());
                }
            }

            /// Configure serif font family.
            fn configure_serif_family(db: &mut $db, available: &std::collections::HashSet<String>) {
                let serif = [
                    "Times New Roman",
                    "DejaVu Serif",
                    "Liberation Serif",
                    "Noto Serif",
                ];
                if let Some(f) = serif.iter().find(|n| available.contains(**n)) {
                    db.set_serif_family(*f);
                }
            }

            /// Configure monospace font family.
            fn configure_monospace_family(
                db: &mut $db,
                available: &std::collections::HashSet<String>,
            ) {
                let mono = [
                    "Courier New",
                    "DejaVu Sans Mono",
                    "Liberation Mono",
                    "Noto Sans Mono",
                ];
                if let Some(f) = mono.iter().find(|n| available.contains(**n)) {
                    db.set_monospace_family(*f);
                }
            }
        }
    };
}

define_fontdb_loader!(resvg_loader, resvg::usvg::fontdb::Database);
define_fontdb_loader!(svg2pdf_loader, svg2pdf::usvg::fontdb::Database);

/// Shared font database for SVG → PNG, built with our own `resvg`'s `fontdb`.
pub(super) fn shared_fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    resvg_loader::shared()
}

/// Shared font database for SVG → PDF, built with `svg2pdf`'s bundled
/// `fontdb` — an older, incompatible version of the one above, so each side
/// of the boundary keeps its own fully populated database.
pub(super) fn shared_svg2pdf_fontdb() -> Arc<svg2pdf::usvg::fontdb::Database> {
    svg2pdf_loader::shared()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loader is a cache, not a rebuild: system font directories are
    /// scanned once and every diagram reuses the same populated database.
    #[test]
    fn shared_font_databases_are_cached_not_built_per_call() {
        assert!(
            Arc::ptr_eq(&shared_fontdb(), &shared_fontdb()),
            "the SVG → PNG fontdb must be built once and reused"
        );
        assert!(
            Arc::ptr_eq(&shared_svg2pdf_fontdb(), &shared_svg2pdf_fontdb()),
            "the SVG → PDF fontdb must be built once and reused"
        );
    }
}
