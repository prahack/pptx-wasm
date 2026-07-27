//! Open Packaging Conventions: the ZIP container, `[Content_Types].xml`, and the
//! `.rels` graph that every other part is reached through.
//!
//! Parts are decompressed lazily and cached. A deck's images are usually most of its
//! bytes and a viewer showing slide 1 has no reason to inflate slide 60's photographs,
//! so `Package` holds the archive open and inflates on first touch.

pub mod content_types;
mod part_name;
mod rels;

pub use content_types::ContentTypes;
pub use part_name::PartName;
pub use rels::{Relationship, Relationships, TargetMode};

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Read};

use crate::error::{Error, Result};

/// Relationship type URIs, minus their shared prefix.
pub mod rel_type {
    pub const OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    pub const SLIDE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
    pub const SLIDE_LAYOUT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
    pub const SLIDE_MASTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
    pub const NOTES_SLIDE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
    pub const THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
    pub const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    pub const CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
    pub const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    pub const FONT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/font";
}

/// A read-only `.pptx` package.
pub struct Package {
    archive: RefCell<zip::ZipArchive<Cursor<Vec<u8>>>>,
    /// Lower-cased part name → index in the archive. OPC part names are
    /// case-insensitive, and real decks in the wild disagree with themselves about
    /// casing between a `.rels` target and the actual entry.
    index: HashMap<String, usize>,
    cache: RefCell<HashMap<String, std::rc::Rc<[u8]>>>,
    rels_cache: RefCell<HashMap<String, std::rc::Rc<Relationships>>>,
    content_types: ContentTypes,
}

impl Package {
    pub fn open(bytes: Vec<u8>) -> Result<Package> {
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| Error::Container(format!("not a zip archive: {e}")))?;

        let mut index = HashMap::with_capacity(archive.len());
        for i in 0..archive.len() {
            let Ok(entry) = archive.by_index_raw(i) else {
                continue;
            };
            if entry.is_dir() {
                continue;
            }
            // `mangled_name` would rewrite `[Content_Types].xml`; OPC names are already
            // constrained, and the archive is never written to disk, so use them as-is.
            let name = PartName::normalize(entry.name());
            index.insert(name.to_ascii_lowercase(), i);
        }

        let package = Package {
            archive: RefCell::new(archive),
            index,
            cache: RefCell::new(HashMap::new()),
            rels_cache: RefCell::new(HashMap::new()),
            content_types: ContentTypes::default(),
        };

        let ct_bytes = package
            .read_part("[Content_Types].xml")
            .ok_or_else(|| Error::Container("no [Content_Types].xml".into()))?;
        let content_types = ContentTypes::parse(&ct_bytes);

        Ok(Package {
            content_types,
            ..package
        })
    }

    /// Inflates a part, or returns `None` if the package has no such part.
    pub fn part(&self, name: &str) -> Option<std::rc::Rc<[u8]>> {
        self.read_part(&PartName::normalize(name))
    }

    fn read_part(&self, normalized: &str) -> Option<std::rc::Rc<[u8]>> {
        let key = normalized.to_ascii_lowercase();
        if let Some(hit) = self.cache.borrow().get(&key) {
            return Some(hit.clone());
        }
        let idx = *self.index.get(&key)?;
        let mut archive = self.archive.borrow_mut();
        let mut entry = archive.by_index(idx).ok()?;
        // Guard against a zip bomb declaring an implausible inflated size.
        const MAX_PART: u64 = 512 * 1024 * 1024;
        if entry.size() > MAX_PART {
            log::warn!("part {normalized} declares {} bytes; refusing", entry.size());
            return None;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        if let Err(e) = entry.read_to_end(&mut buf) {
            log::warn!("part {normalized} failed to inflate: {e}");
            return None;
        }
        let rc: std::rc::Rc<[u8]> = buf.into();
        self.cache.borrow_mut().insert(key, rc.clone());
        Some(rc)
    }

    pub fn has_part(&self, name: &str) -> bool {
        self.index
            .contains_key(&PartName::normalize(name).to_ascii_lowercase())
    }

    /// Every part name in the package, in archive order.
    pub fn part_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.index.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn content_types(&self) -> &ContentTypes {
        &self.content_types
    }

    /// Relationships declared *by* `part_name`. The package root's relationships are
    /// reached with an empty part name.
    pub fn relationships(&self, part_name: &str) -> std::rc::Rc<Relationships> {
        let normalized = PartName::normalize(part_name);
        if let Some(hit) = self.rels_cache.borrow().get(&normalized) {
            return hit.clone();
        }
        let rels_path = PartName::rels_path_for(&normalized);
        let parsed = match self.read_part(&rels_path) {
            Some(bytes) => Relationships::parse(&bytes, &normalized),
            None => Relationships::default(),
        };
        let rc = std::rc::Rc::new(parsed);
        self.rels_cache.borrow_mut().insert(normalized, rc.clone());
        rc
    }

    /// Follows a relationship id from `part_name` to the target part's bytes.
    pub fn resolve_part(&self, part_name: &str, r_id: &str) -> Option<std::rc::Rc<[u8]>> {
        let target = self.resolve_target(part_name, r_id)?;
        self.part(&target)
    }

    /// Follows a relationship id to an absolute part name, without reading it.
    /// Returns `None` for external targets, which a read-only viewer never fetches.
    pub fn resolve_target(&self, part_name: &str, r_id: &str) -> Option<String> {
        let rels = self.relationships(part_name);
        let rel = rels.by_id(r_id)?;
        if rel.target_mode == TargetMode::External {
            return None;
        }
        Some(rel.absolute_target.clone())
    }

    /// The single part related to `part_name` by `rel_type`, if there is exactly one
    /// (slide → layout, layout → master, master → theme all work this way).
    pub fn resolve_single(&self, part_name: &str, rel_type: &str) -> Option<String> {
        let rels = self.relationships(part_name);
        let target = rels.by_type(rel_type).next().map(|r| r.absolute_target.clone());
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a minimal in-memory package. Keeps container tests hermetic — no fixture
    /// files, no python, no disk.
    pub(crate) fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                w.start_file(*name, opts).expect("start_file");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    const CT: &[u8] = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
</Types>"#;

    #[test]
    fn opens_and_reads_a_part() {
        let zip = build_zip(&[("[Content_Types].xml", CT), ("ppt/presentation.xml", b"<p/>")]);
        let pkg = Package::open(zip).expect("open");
        assert_eq!(pkg.part("ppt/presentation.xml").as_deref(), Some(&b"<p/>"[..]));
        assert_eq!(pkg.part("/ppt/presentation.xml").as_deref(), Some(&b"<p/>"[..]));
        assert!(pkg.part("ppt/nope.xml").is_none());
    }

    #[test]
    fn part_lookup_is_case_insensitive() {
        let zip = build_zip(&[("[Content_Types].xml", CT), ("ppt/Slides/Slide1.xml", b"<s/>")]);
        let pkg = Package::open(zip).expect("open");
        assert!(pkg.has_part("ppt/slides/slide1.xml"));
        assert!(pkg.part("PPT/SLIDES/SLIDE1.XML").is_some());
    }

    #[test]
    fn missing_content_types_is_an_error_not_a_panic() {
        let zip = build_zip(&[("ppt/presentation.xml", b"<p/>")]);
        assert!(matches!(Package::open(zip), Err(Error::Container(_))));
    }

    #[test]
    fn garbage_bytes_are_an_error_not_a_panic() {
        assert!(Package::open(b"this is not a zip file at all".to_vec()).is_err());
        assert!(Package::open(Vec::new()).is_err());
    }

    #[test]
    fn parts_are_inflated_once_and_cached() {
        let zip = build_zip(&[("[Content_Types].xml", CT), ("a.xml", b"<a/>")]);
        let pkg = Package::open(zip).expect("open");
        let first = pkg.part("a.xml").expect("first");
        let second = pkg.part("a.xml").expect("second");
        assert!(std::rc::Rc::ptr_eq(&first, &second), "second read should hit the cache");
    }

    #[test]
    fn relationships_resolve_relative_targets() {
        let root_rels = br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#;
        let slide_rels = br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com" TargetMode="External"/>
</Relationships>"#;
        let zip = build_zip(&[
            ("[Content_Types].xml", CT),
            ("_rels/.rels", root_rels),
            ("ppt/presentation.xml", b"<p/>"),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/slideLayouts/slideLayout1.xml", b"<l/>"),
            ("ppt/media/image1.png", b"\x89PNG"),
        ]);
        let pkg = Package::open(zip).expect("open");

        assert_eq!(pkg.resolve_target("", "rId1").as_deref(), Some("ppt/presentation.xml"));
        assert_eq!(
            pkg.resolve_target("ppt/slides/slide1.xml", "rId1").as_deref(),
            Some("ppt/slideLayouts/slideLayout1.xml")
        );
        assert_eq!(
            pkg.resolve_part("ppt/slides/slide1.xml", "rId2").as_deref(),
            Some(&b"\x89PNG"[..])
        );
        // External targets are never fetched.
        assert_eq!(pkg.resolve_target("ppt/slides/slide1.xml", "rId3"), None);
    }

    #[test]
    fn a_part_with_no_rels_file_yields_an_empty_set() {
        let zip = build_zip(&[("[Content_Types].xml", CT), ("ppt/slides/slide1.xml", b"<s/>")]);
        let pkg = Package::open(zip).expect("open");
        assert!(pkg.relationships("ppt/slides/slide1.xml").is_empty());
        assert_eq!(pkg.resolve_target("ppt/slides/slide1.xml", "rId1"), None);
    }
}
