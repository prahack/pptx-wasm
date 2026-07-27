//! Read-only `.pptx` core: OPC container → OOXML parse → presentation model →
//! layout → display list.
//!
//! Two rules hold this crate together and are enforced by review, not the compiler:
//!
//! 1. **The model stays in EMUs.** Conversion to points happens once, in [`layout`],
//!    at the viewport boundary. Nothing downstream of layout sees an EMU.
//! 2. **Layout never touches a canvas.** It emits a [`dl::DisplayList`] and nothing else.
//!    Every pixel is produced by a `Renderer` backend in the `pptx-renderer` crate.
//!
//! Parsing degrades rather than fails: a malformed or unrecognised element is skipped and
//! logged, so a partly-broken deck still renders everything it can.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(unsafe_code)]
#![warn(clippy::indexing_slicing)]

pub mod dl;
pub mod emu;
pub mod error;
pub mod layout;
pub mod model;
pub mod opc;
pub mod parse;
pub mod text;

pub use error::{Error, Result};

/// The parsed presentation plus everything needed to lay any slide out on demand.
pub use model::Presentation;

/// Opens a `.pptx` from its raw bytes.
///
/// Only the package index and `presentation.xml` are read eagerly; slide bodies are
/// parsed on first access so that opening a 200-slide deck stays cheap.
pub fn open(bytes: Vec<u8>) -> Result<Presentation> {
    let package = opc::Package::open(bytes)?;
    parse::presentation::parse(package)
}
