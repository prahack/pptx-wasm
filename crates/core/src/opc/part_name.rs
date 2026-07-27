//! OPC part-name normalisation and relative-target resolution.
//!
//! Part names in a package are absolute and rooted, but `.rels` targets are relative to
//! the *directory* of the part that declares them, and real decks emit a mix of `../`,
//! `./`, absolute `/ppt/...`, and percent-encoded forms. Getting this wrong shows up as
//! a silently missing image, so it lives in one place with its own tests.

pub struct PartName;

impl PartName {
    /// Canonical internal form: no leading slash, forward slashes, no `.`/`..` segments,
    /// percent-decoded.
    pub fn normalize(name: &str) -> String {
        let decoded = percent_decode(name.trim());
        let replaced = decoded.replace('\\', "/");
        let mut out: Vec<&str> = Vec::new();
        for seg in replaced.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                s => out.push(s),
            }
        }
        out.join("/")
    }

    /// The `.rels` part that carries `part`'s relationships.
    ///
    /// `ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`, and the package
    /// root (empty name) → `_rels/.rels`.
    pub fn rels_path_for(part: &str) -> String {
        let part = Self::normalize(part);
        if part.is_empty() {
            return "_rels/.rels".to_string();
        }
        match part.rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
            None => format!("_rels/{part}.rels"),
        }
    }

    /// Resolves a `.rels` `Target` against the part that declared it.
    pub fn resolve_target(source_part: &str, target: &str) -> String {
        let target = target.trim();
        if target.starts_with('/') {
            return Self::normalize(target);
        }
        let source = Self::normalize(source_part);
        let dir = match source.rsplit_once('/') {
            Some((d, _)) => d,
            None => "",
        };
        if dir.is_empty() {
            Self::normalize(target)
        } else {
            Self::normalize(&format!("{dir}/{target}"))
        }
    }

    /// Lower-cased extension without the dot.
    pub fn extension(part: &str) -> Option<String> {
        let file = part.rsplit('/').next()?;
        let (_, ext) = file.rsplit_once('.')?;
        Some(ext.to_ascii_lowercase())
    }
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'%') if i + 2 < bytes.len() => {
                let hex = s.get(i + 1..i + 3).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            Some(&b) => {
                out.push(b);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_leading_slash_and_dot_segments() {
        assert_eq!(
            PartName::normalize("/ppt/presentation.xml"),
            "ppt/presentation.xml"
        );
        assert_eq!(
            PartName::normalize("ppt/./slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            PartName::normalize("ppt/slides/../media/i.png"),
            "ppt/media/i.png"
        );
        assert_eq!(PartName::normalize(""), "");
    }

    #[test]
    fn normalize_decodes_percent_escapes() {
        assert_eq!(
            PartName::normalize("ppt/media/my%20image.png"),
            "ppt/media/my image.png"
        );
        // A stray percent is left alone rather than eating the next character.
        assert_eq!(PartName::normalize("ppt/100%.xml"), "ppt/100%.xml");
    }

    #[test]
    fn rels_path_puts_the_underscore_rels_directory_beside_the_part() {
        assert_eq!(
            PartName::rels_path_for("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
        assert_eq!(
            PartName::rels_path_for("ppt/presentation.xml"),
            "ppt/_rels/presentation.xml.rels"
        );
        assert_eq!(PartName::rels_path_for(""), "_rels/.rels");
    }

    #[test]
    fn targets_resolve_against_the_declaring_parts_directory() {
        assert_eq!(
            PartName::resolve_target("ppt/slides/slide1.xml", "../slideLayouts/slideLayout1.xml"),
            "ppt/slideLayouts/slideLayout1.xml"
        );
        assert_eq!(
            PartName::resolve_target("ppt/slides/slide1.xml", "slide2.xml"),
            "ppt/slides/slide2.xml"
        );
        // Root-relative targets ignore the source directory entirely.
        assert_eq!(
            PartName::resolve_target("ppt/slides/slide1.xml", "/ppt/media/i.png"),
            "ppt/media/i.png"
        );
        // Targets declared by the package root.
        assert_eq!(
            PartName::resolve_target("", "ppt/presentation.xml"),
            "ppt/presentation.xml"
        );
    }

    #[test]
    fn escaping_above_the_package_root_clamps_instead_of_underflowing() {
        assert_eq!(
            PartName::resolve_target("a.xml", "../../../etc/passwd"),
            "etc/passwd"
        );
    }

    #[test]
    fn extension_is_lowercased() {
        assert_eq!(
            PartName::extension("ppt/media/i.PNG").as_deref(),
            Some("png")
        );
        assert_eq!(PartName::extension("ppt/media/noext"), None);
    }
}
