//! OOXML parsers. Every function here degrades: unknown elements are skipped, malformed
//! values fall back to a documented default, and nothing panics on hostile input.

pub mod drawing;
pub mod presentation;
pub mod shapes;
pub mod slide;
pub mod table;
pub mod table_style;
pub mod text;
pub mod theme;
pub mod xml;
