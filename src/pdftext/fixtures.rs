//! PDF fixtures embedded into the test binary (see `super::tests`).

pub(super) const LIGATURES_PDF: &[u8] = include_bytes!("../../tests/fixtures/ligatures.pdf");
pub(super) const PAGES_PDF: &[u8] = include_bytes!("../../tests/fixtures/pages-ligatures.pdf");
pub(super) const MALFORMED_DATE_PDF: &[u8] =
    include_bytes!("../../tests/fixtures/malformed-date.pdf");

pub(super) const CAPABILITIES_PDF: &[u8] =
    include_bytes!("../../examples/texforge-capabilites/texforge-capabilites.pdf");
