//! The presentation model. Everything here is in EMUs and unresolved property terms;
//! nothing knows about pixels, canvases, or the theme a given shape will end up under.

pub mod chart;
pub mod color;
pub mod fill;
pub mod geometry;
pub mod media;
pub mod shape;
pub mod table;
pub mod table_style;
pub mod text;
pub mod theme;

pub use color::{ColorMod, ColorRef, ColorSpec, SchemeColor};
pub use fill::{Effects, Fill, Line};
pub use geometry::{Geometry, GroupTransform, Transform2D};
pub use media::{EmbeddedFont, MediaEntry, MediaRegistry};
pub use shape::{
    Background, ColorMap, Placeholder, PlaceholderType, Shape, ShapeKind, ShapeTree, Slide,
    SlideLayout, SlideMaster,
};
pub use chart::Chart;
pub use table::{Table, TableCell, TableRow};
pub use table_style::{CellPosition, PartStyle, TableStyle, TableStyles};
pub use text::{BodyProps, ListStyle, Paragraph, ParagraphProps, Run, RunProps, TextBody};
pub use theme::{ColorScheme, FontScheme, FormatScheme, Theme};

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::emu::Emu;
use crate::opc::Package;

/// A slide's identity in the deck, cheap to hand across the wasm boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideInfo {
    /// 0-based position in presentation order.
    pub index: usize,
    /// `<p:sldId id="...">`, stable across reorderings.
    pub id: u32,
    pub part_name: String,
}

/// An open presentation.
///
/// Only `presentation.xml` and the package index are read when this is constructed.
/// Slides, layouts, masters and themes are parsed on first access and cached, so opening
/// a 300-slide deck to show slide 1 costs one slide's worth of parsing.
pub struct Presentation {
    pub(crate) package: Package,
    /// Slide size in EMUs.
    pub slide_width: Emu,
    pub slide_height: Emu,
    pub(crate) slides: Vec<SlideInfo>,
    /// `<p:defaultTextStyle>` — the bottom of the text inheritance chain.
    pub default_text_style: ListStyle,
    pub embedded_fonts: Vec<EmbeddedFont>,
    /// Part name of the first master, used when a slide's chain is broken.
    pub(crate) first_master: Option<String>,

    pub(crate) slide_cache: RefCell<HashMap<String, Rc<Slide>>>,
    pub(crate) layout_cache: RefCell<HashMap<String, Rc<SlideLayout>>>,
    pub(crate) master_cache: RefCell<HashMap<String, Rc<SlideMaster>>>,
    pub(crate) theme_cache: RefCell<HashMap<String, Rc<Theme>>>,
    pub(crate) media: RefCell<MediaRegistry>,
    /// `ppt/tableStyles.xml`, parsed on first use. Decks without a table never pay for it.
    pub(crate) table_styles: RefCell<Option<Rc<TableStyles>>>,
    pub(crate) chart_cache: RefCell<HashMap<String, Rc<Chart>>>,
}

impl Presentation {
    pub(crate) fn new(package: Package, slide_width: Emu, slide_height: Emu) -> Self {
        Presentation {
            package,
            slide_width,
            slide_height,
            slides: Vec::new(),
            default_text_style: ListStyle::default(),
            embedded_fonts: Vec::new(),
            first_master: None,
            slide_cache: RefCell::new(HashMap::new()),
            layout_cache: RefCell::new(HashMap::new()),
            master_cache: RefCell::new(HashMap::new()),
            theme_cache: RefCell::new(HashMap::new()),
            media: RefCell::new(MediaRegistry::new()),
            table_styles: RefCell::new(None),
            chart_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Parses (or returns the cached) chart part.
    pub fn chart(&self, part: &str) -> Option<Rc<Chart>> {
        if let Some(hit) = self.chart_cache.borrow().get(part) {
            return Some(Rc::clone(hit));
        }
        let bytes = self.package.part(part)?;
        let parsed = Rc::new(crate::parse::chart::parse_chart(&bytes));
        self.chart_cache
            .borrow_mut()
            .insert(part.to_string(), Rc::clone(&parsed));
        Some(parsed)
    }

    /// The deck's table styles.
    ///
    /// Almost always just a default GUID pointing at a style PowerPoint keeps to itself,
    /// which [`TableStyles::get`] resolves against the built-in table.
    pub fn table_styles(&self) -> Rc<TableStyles> {
        if let Some(hit) = self.table_styles.borrow().as_ref() {
            return Rc::clone(hit);
        }
        let parsed = match self.package.part("ppt/tableStyles.xml") {
            Some(bytes) => crate::parse::table_style::parse_table_styles(&bytes),
            None => TableStyles::default(),
        };
        let rc = Rc::new(parsed);
        *self.table_styles.borrow_mut() = Some(Rc::clone(&rc));
        rc
    }

    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    pub fn slides(&self) -> &[SlideInfo] {
        &self.slides
    }

    pub fn slide_info(&self, index: usize) -> Option<&SlideInfo> {
        self.slides.get(index)
    }

    /// Slide size in points.
    pub fn slide_size_pt(&self) -> (f32, f32) {
        (
            crate::emu::to_pt(self.slide_width),
            crate::emu::to_pt(self.slide_height),
        )
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Parses (or returns the cached) slide at `index`.
    pub fn slide(&self, index: usize) -> Option<Rc<Slide>> {
        let info = self.slides.get(index)?;
        self.slide_by_part(&info.part_name)
    }

    pub fn slide_by_part(&self, part: &str) -> Option<Rc<Slide>> {
        if let Some(hit) = self.slide_cache.borrow().get(part) {
            return Some(hit.clone());
        }
        let bytes = self.package.part(part)?;
        let parsed = Rc::new(crate::parse::slide::parse_slide(self, part, &bytes));
        self.slide_cache
            .borrow_mut()
            .insert(part.to_string(), parsed.clone());
        Some(parsed)
    }

    pub fn layout(&self, part: &str) -> Option<Rc<SlideLayout>> {
        if let Some(hit) = self.layout_cache.borrow().get(part) {
            return Some(hit.clone());
        }
        let bytes = self.package.part(part)?;
        let parsed = Rc::new(crate::parse::slide::parse_layout(self, part, &bytes));
        self.layout_cache
            .borrow_mut()
            .insert(part.to_string(), parsed.clone());
        Some(parsed)
    }

    pub fn master(&self, part: &str) -> Option<Rc<SlideMaster>> {
        if let Some(hit) = self.master_cache.borrow().get(part) {
            return Some(hit.clone());
        }
        let bytes = self.package.part(part)?;
        let parsed = Rc::new(crate::parse::slide::parse_master(self, part, &bytes));
        self.master_cache
            .borrow_mut()
            .insert(part.to_string(), parsed.clone());
        Some(parsed)
    }

    pub fn theme(&self, part: &str) -> Option<Rc<Theme>> {
        if let Some(hit) = self.theme_cache.borrow().get(part) {
            return Some(hit.clone());
        }
        let bytes = self.package.part(part)?;
        let parsed = Rc::new(crate::parse::theme::parse_theme(&bytes));
        self.theme_cache
            .borrow_mut()
            .insert(part.to_string(), parsed.clone());
        Some(parsed)
    }

    /// The full inheritance chain for a slide: itself, its layout, its master, its theme.
    ///
    /// Any link can be missing — decks with a slide whose layout was deleted do exist —
    /// and each fallback is chosen so the slide still renders: no layout means the
    /// master alone, no master means the first master in the deck, no theme means the
    /// Office default.
    pub fn chain_for(&self, index: usize) -> Option<SlideChain> {
        let slide = self.slide(index)?;
        let layout = slide
            .layout_part
            .as_deref()
            .and_then(|p| self.layout(p));
        let master = layout
            .as_ref()
            .and_then(|l| l.master_part.as_deref())
            .or(self.first_master.as_deref())
            .and_then(|p| self.master(p));
        let theme = master
            .as_ref()
            .and_then(|m| m.theme_part.as_deref())
            .and_then(|p| self.theme(p))
            .unwrap_or_else(|| Rc::new(Theme::default()));
        Some(SlideChain {
            slide,
            layout,
            master,
            theme,
        })
    }

    /// Interns an image part reached by relationship id from `source_part`.
    pub fn intern_image(&self, source_part: &str, r_id: &str) -> Option<crate::dl::ImageId> {
        let target = self.package.resolve_target(source_part, r_id)?;
        let mime = self.package.content_types().image_mime(&target);
        Some(self.media.borrow_mut().intern(&target, mime))
    }

    /// Bytes of an interned image.
    pub fn image_bytes(&self, id: crate::dl::ImageId) -> Option<Rc<[u8]>> {
        let part = self.media.borrow().get(id)?.part_name.clone();
        self.package.part(&part)
    }

    pub fn image_entry(&self, id: crate::dl::ImageId) -> Option<MediaEntry> {
        self.media.borrow().get(id).cloned()
    }

    pub fn media_count(&self) -> usize {
        self.media.borrow().len()
    }

    /// Drops parsed slides but keeps layouts, masters and themes — they are small,
    /// shared by every slide, and expensive to re-derive.
    pub fn evict_slide_cache(&self) {
        self.slide_cache.borrow_mut().clear();
    }
}

/// A slide together with everything it inherits from.
pub struct SlideChain {
    pub slide: Rc<Slide>,
    pub layout: Option<Rc<SlideLayout>>,
    pub master: Option<Rc<SlideMaster>>,
    pub theme: Rc<Theme>,
}

impl SlideChain {
    /// The colour map in force, which comes from the master.
    pub fn color_map(&self) -> ColorMap {
        self.master.as_ref().map(|m| m.color_map).unwrap_or_default()
    }

    /// Resolves a colour reference in this slide's context.
    pub fn resolve_color(&self, c: &ColorRef, placeholder: Option<crate::dl::Color>) -> crate::dl::Color {
        self.theme.resolve_color(c, self.color_map(), placeholder)
    }

    /// The effective background, walking slide → layout → master.
    pub fn background(&self) -> &Background {
        if self.slide.background != Background::Inherit {
            return &self.slide.background;
        }
        if let Some(l) = &self.layout {
            if l.background != Background::Inherit {
                return &l.background;
            }
        }
        if let Some(m) = &self.master {
            return &m.background;
        }
        &Background::Inherit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_size_converts_to_points() {
        let pkg = crate::opc::Package::open(minimal_zip()).expect("open");
        let p = Presentation::new(pkg, 12_192_000, 6_858_000);
        assert_eq!(p.slide_size_pt(), (960.0, 540.0));
    }

    #[test]
    fn an_empty_deck_reports_no_slides_without_panicking() {
        let pkg = crate::opc::Package::open(minimal_zip()).expect("open");
        let p = Presentation::new(pkg, 1, 1);
        assert_eq!(p.slide_count(), 0);
        assert!(p.slide(0).is_none());
        assert!(p.slide_info(0).is_none());
        assert!(p.chain_for(0).is_none());
    }

    fn minimal_zip() -> Vec<u8> {
        use std::io::{Cursor, Write};
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("[Content_Types].xml", opts).expect("start");
            w.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#)
                .expect("write");
            w.finish().expect("finish");
        }
        buf
    }
}
