use std::collections::HashMap;

use quick_xml::events::Event;

use super::part_name::PartName;
use crate::parse::xml::{attr, local_name, Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMode {
    #[default]
    Internal,
    /// A URL. A read-only offline viewer resolves these for display only; it never
    /// fetches them, which is what keeps "no network calls in the core" true.
    External,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    /// As written in the XML.
    pub target: String,
    /// Normalised, package-absolute. Meaningless for external targets.
    pub absolute_target: String,
    pub target_mode: TargetMode,
}

#[derive(Debug, Clone, Default)]
pub struct Relationships {
    by_id: HashMap<String, Relationship>,
    ordered: Vec<String>,
}

impl Relationships {
    /// `source_part` is the part these relationships belong to; targets resolve against
    /// its directory.
    pub fn parse(xml: &[u8], source_part: &str) -> Relationships {
        let mut rels = Relationships::default();
        let mut reader = Reader::new(xml);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                    if local_name(e.name().as_ref()) != b"Relationship" {
                        continue;
                    }
                    let id = attr(&e, b"Id").unwrap_or_default();
                    if id.is_empty() {
                        log::warn!("relationship without Id in {source_part}, skipping");
                        continue;
                    }
                    let target = attr(&e, b"Target").unwrap_or_default();
                    let target_mode = match attr(&e, b"TargetMode").as_deref() {
                        Some("External") => TargetMode::External,
                        _ => TargetMode::Internal,
                    };
                    let absolute_target = match target_mode {
                        TargetMode::Internal => PartName::resolve_target(source_part, &target),
                        TargetMode::External => String::new(),
                    };
                    let rel = Relationship {
                        id: id.clone(),
                        rel_type: attr(&e, b"Type").unwrap_or_default(),
                        target,
                        absolute_target,
                        target_mode,
                    };
                    if rels.by_id.insert(id.clone(), rel).is_none() {
                        rels.ordered.push(id);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    // A truncated .rels costs us the parts it referenced, not the deck.
                    log::warn!("malformed relationships for {source_part}: {e}");
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        rels
    }

    pub fn by_id(&self, id: &str) -> Option<&Relationship> {
        self.by_id.get(id)
    }

    /// All relationships of a type, in document order.
    pub fn by_type<'a>(&'a self, rel_type: &'a str) -> impl Iterator<Item = &'a Relationship> + 'a {
        self.ordered
            .iter()
            .filter_map(move |id| self.by_id.get(id))
            .filter(move |r| r.rel_type == rel_type)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.ordered.iter().filter_map(|id| self.by_id.get(id))
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::rel_type;

    const XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image2.png"/>
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout2.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;

    #[test]
    fn parses_ids_types_and_absolute_targets() {
        let r = Relationships::parse(XML, "ppt/slides/slide1.xml");
        assert_eq!(r.len(), 3);
        let rel = r.by_id("rId1").expect("rId1");
        assert_eq!(rel.rel_type, rel_type::SLIDE_LAYOUT);
        assert_eq!(rel.absolute_target, "ppt/slideLayouts/slideLayout2.xml");
    }

    #[test]
    fn by_type_preserves_document_order_not_id_order() {
        let r = Relationships::parse(XML, "ppt/slides/slide1.xml");
        let images: Vec<_> = r.by_type(rel_type::IMAGE).map(|r| r.id.as_str()).collect();
        assert_eq!(images, vec!["rId3", "rId2"]);
    }

    #[test]
    fn truncated_xml_keeps_what_it_managed_to_read() {
        let truncated = br#"<Relationships><Relationship Id="rId1" Type="t" Target="a.xml"/><Relationship Id="rId2"#;
        let r = Relationships::parse(truncated, "ppt/presentation.xml");
        assert_eq!(r.len(), 1);
        assert!(r.by_id("rId1").is_some());
    }

    #[test]
    fn relationships_without_an_id_are_skipped() {
        let xml = br#"<Relationships><Relationship Type="t" Target="a.xml"/></Relationships>"#;
        assert!(Relationships::parse(xml, "").is_empty());
    }

    #[test]
    fn empty_input_yields_an_empty_set() {
        assert!(Relationships::parse(b"", "").is_empty());
        assert!(Relationships::parse(b"not xml at all", "").is_empty());
    }
}
