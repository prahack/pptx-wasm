use std::collections::HashMap;

use quick_xml::events::Event;

use super::part_name::PartName;
use crate::parse::xml::{attr, local_name, Reader};

/// `[Content_Types].xml` — extension defaults plus per-part overrides.
///
/// Used to tell an image's real format from its bytes' provenance (a `.bin` part can be
/// a PNG) and to spot the presentation part without guessing at its name.
#[derive(Debug, Clone, Default)]
pub struct ContentTypes {
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}

impl ContentTypes {
    pub fn parse(xml: &[u8]) -> ContentTypes {
        let mut ct = ContentTypes::default();
        let mut reader = Reader::new(xml);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                    b"Default" => {
                        if let (Some(ext), Some(t)) =
                            (attr(&e, b"Extension"), attr(&e, b"ContentType"))
                        {
                            ct.defaults.insert(ext.to_ascii_lowercase(), t);
                        }
                    }
                    b"Override" => {
                        if let (Some(part), Some(t)) =
                            (attr(&e, b"PartName"), attr(&e, b"ContentType"))
                        {
                            ct.overrides
                                .insert(PartName::normalize(&part).to_ascii_lowercase(), t);
                        }
                    }
                    _ => {}
                },
                Ok(Event::Eof) => break,
                Err(e) => {
                    log::warn!("malformed [Content_Types].xml: {e}");
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        ct
    }

    /// Content type for a part: an override if one exists, else the extension default.
    pub fn for_part(&self, part: &str) -> Option<&str> {
        let normalized = PartName::normalize(part).to_ascii_lowercase();
        if let Some(t) = self.overrides.get(&normalized) {
            return Some(t);
        }
        let ext = PartName::extension(&normalized)?;
        self.defaults.get(&ext).map(String::as_str)
    }

    /// Parts carrying a given content type, sorted for determinism.
    pub fn parts_with_type(&self, content_type: &str) -> Vec<String> {
        let mut parts: Vec<String> = self
            .overrides
            .iter()
            .filter(|(_, t)| t.as_str() == content_type)
            .map(|(p, _)| p.clone())
            .collect();
        parts.sort();
        parts
    }

    /// MIME type suitable for a `data:` URL or an `ImageBitmap` blob.
    ///
    /// Falls back to sniffing the extension, because a surprising number of decks
    /// declare `image/x-emf` or nothing at all for perfectly ordinary PNGs.
    pub fn image_mime(&self, part: &str) -> &'static str {
        let declared = self.for_part(part).unwrap_or_default().to_ascii_lowercase();
        let by_declared = match declared.as_str() {
            "image/png" => Some("image/png"),
            "image/jpeg" | "image/jpg" => Some("image/jpeg"),
            "image/gif" => Some("image/gif"),
            "image/bmp" | "image/x-ms-bmp" => Some("image/bmp"),
            "image/webp" => Some("image/webp"),
            "image/svg+xml" => Some("image/svg+xml"),
            "image/tiff" => Some("image/tiff"),
            _ => None,
        };
        if let Some(m) = by_declared {
            return m;
        }
        match PartName::extension(part).as_deref() {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") | Some("jfif") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("bmp") => "image/bmp",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            Some("tif") | Some("tiff") => "image/tiff",
            // EMF/WMF are vector formats no browser decodes. Named honestly so the
            // renderer can draw a placeholder instead of a broken-image box.
            Some("emf") => "image/x-emf",
            Some("wmf") => "image/x-wmf",
            _ => "application/octet-stream",
        }
    }
}

/// Content types worth naming.
pub mod content_type {
    pub const PRESENTATION: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
    pub const SLIDESHOW: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml";
    pub const TEMPLATE: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml";
    pub const SLIDE: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
    pub const SLIDE_LAYOUT: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
    pub const SLIDE_MASTER: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
    pub const THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
    pub const CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &[u8] = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="PNG" ContentType="image/png"/>
  <Default Extension="bin" ContentType="application/octet-stream"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/media/logo.bin" ContentType="image/jpeg"/>
</Types>"#;

    #[test]
    fn overrides_beat_extension_defaults() {
        let ct = ContentTypes::parse(XML);
        assert_eq!(ct.for_part("/ppt/media/logo.bin"), Some("image/jpeg"));
        assert_eq!(
            ct.for_part("ppt/other.bin"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn extension_matching_ignores_case_on_both_sides() {
        let ct = ContentTypes::parse(XML);
        assert_eq!(ct.for_part("ppt/media/image1.png"), Some("image/png"));
        assert_eq!(ct.for_part("ppt/media/IMAGE1.PNG"), Some("image/png"));
    }

    #[test]
    fn finds_the_presentation_part_without_guessing_its_name() {
        let ct = ContentTypes::parse(XML);
        assert_eq!(
            ct.parts_with_type(content_type::PRESENTATION),
            vec!["ppt/presentation.xml".to_string()]
        );
    }

    #[test]
    fn image_mime_falls_back_to_the_extension_when_the_declaration_is_useless() {
        let ct = ContentTypes::parse(XML);
        // Declared octet-stream, but the extension is authoritative enough to use.
        assert_eq!(ct.image_mime("ppt/media/image3.jpeg"), "image/jpeg");
        // Declared correctly despite a misleading extension.
        assert_eq!(ct.image_mime("ppt/media/logo.bin"), "image/jpeg");
        // Vector metafiles are named rather than passed off as something decodable.
        assert_eq!(ct.image_mime("ppt/media/chart.emf"), "image/x-emf");
    }

    #[test]
    fn empty_or_broken_input_yields_an_empty_map_not_a_panic() {
        assert_eq!(ContentTypes::parse(b"").for_part("a.xml"), None);
        assert_eq!(
            ContentTypes::parse(b"<Types><Default").for_part("a.png"),
            None
        );
    }
}
