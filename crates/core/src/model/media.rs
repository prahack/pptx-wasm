//! The media registry: a stable [`ImageId`] per image part.
//!
//! The display list refers to images by id rather than by bytes or part name. That keeps
//! commands small and cheap to clone, lets the renderer keep a decoded-bitmap cache keyed
//! by something it can compare in one instruction, and means the same photo used on
//! twenty slides is decoded once.

use std::collections::HashMap;

use crate::dl::ImageId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    pub part_name: String,
    /// MIME type, resolved through `[Content_Types].xml` with an extension fallback.
    pub mime: &'static str,
}

impl MediaEntry {
    /// Whether a browser can decode this directly. EMF/WMF cannot be, and the renderer
    /// draws a placeholder rather than a broken image.
    pub fn is_browser_decodable(&self) -> bool {
        matches!(
            self.mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/webp" | "image/svg+xml"
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct MediaRegistry {
    by_part: HashMap<String, ImageId>,
    entries: Vec<MediaEntry>,
}

impl MediaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a part, returning its existing id if it is already known. Ids are
    /// assigned in first-use order and never reused.
    pub fn intern(&mut self, part_name: &str, mime: &'static str) -> ImageId {
        if let Some(id) = self.by_part.get(part_name) {
            return *id;
        }
        let id = ImageId(self.entries.len() as u32);
        self.entries.push(MediaEntry {
            part_name: part_name.to_string(),
            mime,
        });
        self.by_part.insert(part_name.to_string(), id);
        id
    }

    pub fn get(&self, id: ImageId) -> Option<&MediaEntry> {
        self.entries.get(id.0 as usize)
    }

    pub fn id_for_part(&self, part_name: &str) -> Option<ImageId> {
        self.by_part.get(part_name).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ImageId, &MediaEntry)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (ImageId(i as u32), e))
    }
}

/// A font embedded in the deck (`<p:embeddedFontLst>`), which the viewer can install as
/// a `FontFace` so text renders in the authored face even on a machine without it.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedFont {
    pub typeface: String,
    /// Relationship id of the `fntdata` part for each style present.
    pub regular: Option<String>,
    pub bold: Option<String>,
    pub italic: Option<String>,
    pub bold_italic: Option<String>,
}

impl EmbeddedFont {
    pub fn new(typeface: impl Into<String>) -> Self {
        EmbeddedFont {
            typeface: typeface.into(),
            regular: None,
            bold: None,
            italic: None,
            bold_italic: None,
        }
    }

    /// The relationship ids and the (bold, italic) they provide.
    pub fn variants(&self) -> Vec<(&str, bool, bool)> {
        let mut out = Vec::new();
        if let Some(r) = &self.regular {
            out.push((r.as_str(), false, false));
        }
        if let Some(r) = &self.bold {
            out.push((r.as_str(), true, false));
        }
        if let Some(r) = &self.italic {
            out.push((r.as_str(), false, true));
        }
        if let Some(r) = &self.bold_italic {
            out.push((r.as_str(), true, true));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_the_same_part_twice_yields_one_id() {
        let mut r = MediaRegistry::new();
        let a = r.intern("ppt/media/image1.png", "image/png");
        let b = r.intern("ppt/media/image1.png", "image/png");
        assert_eq!(a, b);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn ids_are_assigned_in_first_use_order() {
        let mut r = MediaRegistry::new();
        assert_eq!(r.intern("a.png", "image/png"), ImageId(0));
        assert_eq!(r.intern("b.jpg", "image/jpeg"), ImageId(1));
        assert_eq!(r.intern("a.png", "image/png"), ImageId(0));
        assert_eq!(r.get(ImageId(1)).map(|e| e.mime), Some("image/jpeg"));
        assert!(r.get(ImageId(9)).is_none());
    }

    #[test]
    fn metafiles_are_flagged_as_undecodable() {
        let mut r = MediaRegistry::new();
        let emf = r.intern("ppt/media/image1.emf", "image/x-emf");
        let png = r.intern("ppt/media/image2.png", "image/png");
        assert!(!r.get(emf).map(|e| e.is_browser_decodable()).unwrap_or(true));
        assert!(r
            .get(png)
            .map(|e| e.is_browser_decodable())
            .unwrap_or(false));
    }

    #[test]
    fn embedded_font_variants_list_only_what_is_present() {
        let mut f = EmbeddedFont::new("Corporate Sans");
        f.regular = Some("rId1".into());
        f.bold_italic = Some("rId4".into());
        assert_eq!(
            f.variants(),
            vec![("rId1", false, false), ("rId4", true, true)]
        );
    }
}
