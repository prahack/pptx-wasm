//! Thin helpers over `quick-xml`.
//!
//! **Namespace policy.** Elements are matched on their *local* name: OOXML pins its
//! prefixes so hard in practice (`p:`, `a:`, `r:`, `c:`) that carrying URIs through
//! every match costs more than it buys, and the few elements sharing a local name across
//! namespaces — `txBody`, `blipFill` — mean the same thing in both.
//!
//! Attributes are different. `<p:sldId id="256" r:id="rId2"/>` has two attributes whose
//! local name is `id` and they mean entirely different things, so attribute lookup is
//! prefix-exact by default and the `r:`-namespace ones go through [`r_attr`].

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};

/// A configured reader. OOXML content is whitespace-significant inside `<a:t>`, so
/// text trimming stays off and callers trim where the schema allows it.
pub struct Reader<'a>(quick_xml::Reader<&'a [u8]>);

impl<'a> Reader<'a> {
    pub fn new(xml: &'a [u8]) -> Self {
        let mut r = quick_xml::Reader::from_reader(xml);
        let cfg = r.config_mut();
        cfg.trim_text(false);
        // Decks in the wild ship unbalanced end tags; recovering beats refusing to open.
        cfg.check_end_names = false;
        cfg.expand_empty_elements = false;
        Reader(r)
    }

    pub fn read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<Event<'b>> {
        self.0.read_event_into(buf)
    }

    /// Byte offset reached so far — used in log messages to locate malformed content.
    pub fn position(&self) -> u64 {
        self.0.buffer_position()
    }
}

/// Strips any namespace prefix: `a:solidFill` → `solidFill`.
#[inline]
pub fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().rposition(|&b| b == b':') {
        Some(i) => qname.get(i + 1..).unwrap_or(qname),
        None => qname,
    }
}

/// True when the element's local name matches.
#[inline]
pub fn is(e: &BytesStart<'_>, name: &[u8]) -> bool {
    local_name(e.name().as_ref()) == name
}

fn decode(a: &Attribute<'_>) -> Option<String> {
    match a.unescape_value() {
        Ok(v) => Some(v.into_owned()),
        Err(_) => {
            // Undecodable entity: fall back to the raw bytes rather than dropping the
            // attribute, since most such values are still usable (e.g. a font name).
            Some(String::from_utf8_lossy(&a.value).into_owned())
        }
    }
}

/// Attribute by exact qualified name. Unprefixed names match only unprefixed attributes.
pub fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| decode(&a))
}

/// Attribute in the relationships namespace, e.g. `r:id`, `r:embed`, `r:link`.
///
/// Matched by local name plus the presence of *some* prefix, because the only prefixed
/// attributes OOXML puts on these elements are the relationship ones.
pub fn r_attr(e: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| {
            let key = a.key.as_ref();
            local_name(key) == local && key.len() > local.len()
        })
        .and_then(|a| decode(&a))
}

pub fn attr_i64(e: &BytesStart<'_>, name: &[u8]) -> Option<i64> {
    attr(e, name)?.trim().parse().ok()
}

pub fn attr_i32(e: &BytesStart<'_>, name: &[u8]) -> Option<i32> {
    attr(e, name)?.trim().parse().ok()
}

pub fn attr_u32(e: &BytesStart<'_>, name: &[u8]) -> Option<u32> {
    attr(e, name)?.trim().parse().ok()
}

pub fn attr_f32(e: &BytesStart<'_>, name: &[u8]) -> Option<f32> {
    attr(e, name)?.trim().parse().ok()
}

/// OOXML booleans are `1`/`0` or `true`/`false`; a present-but-empty attribute means true.
pub fn attr_bool(e: &BytesStart<'_>, name: &[u8]) -> Option<bool> {
    let v = attr(e, name)?;
    match v.trim() {
        "1" | "true" | "True" | "on" | "" => Some(true),
        "0" | "false" | "False" | "off" => Some(false),
        _ => None,
    }
}

/// A percentage attribute. OOXML writes these either as 1000ths of a percent
/// (`val="50000"` = 50%) or, since the 2010 extensions, with an explicit `%` sign
/// (`val="50%"`). Returns a 0..1 fraction.
pub fn attr_percent(e: &BytesStart<'_>, name: &[u8]) -> Option<f32> {
    let raw = attr(e, name)?;
    let raw = raw.trim();
    if let Some(stripped) = raw.strip_suffix('%') {
        return stripped.parse::<f32>().ok().map(|v| v / 100.0);
    }
    raw.parse::<f32>().ok().map(|v| v / 100_000.0)
}

/// Concatenated text content of the element the reader has just entered, consuming up to
/// and including its end tag. Nested elements' text is included; their tags are not.
pub fn text_content(reader: &mut Reader<'_>, element: &[u8]) -> String {
    let mut out = String::new();
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == element {
                    depth += 1;
                }
            }
            Ok(Event::Text(t)) => match t.unescape() {
                Ok(s) => out.push_str(&s),
                // A bad entity reference costs that fragment its escapes, not the run.
                Err(_) => out.push_str(&String::from_utf8_lossy(&t)),
            },
            Ok(Event::CData(t)) => {
                out.push_str(&String::from_utf8_lossy(&t));
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == element {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Consumes events until the end tag of `element` at the current nesting level.
/// Used to skip subtrees this build does not understand without losing our place.
pub fn skip_element(reader: &mut Reader<'_>, element: &[u8]) {
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == element => depth += 1,
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == element => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(xml: &str) -> BytesStart<'static> {
        let mut r = Reader::new(xml.as_bytes());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => return e.into_owned(),
                Ok(Event::Eof) => panic!("no element in {xml}"),
                _ => {}
            }
        }
    }

    #[test]
    fn local_name_strips_the_prefix() {
        assert_eq!(local_name(b"a:solidFill"), b"solidFill");
        assert_eq!(local_name(b"sp"), b"sp");
        assert_eq!(local_name(b":odd"), b"odd");
    }

    #[test]
    fn prefixed_and_unprefixed_id_are_told_apart() {
        let e = start(r#"<p:sldId id="256" r:id="rId2"/>"#);
        assert_eq!(attr(&e, b"id").as_deref(), Some("256"));
        assert_eq!(r_attr(&e, b"id").as_deref(), Some("rId2"));
    }

    #[test]
    fn r_attr_finds_embed_and_link() {
        let e = start(r#"<a:blip r:embed="rId3"/>"#);
        assert_eq!(r_attr(&e, b"embed").as_deref(), Some("rId3"));
        assert_eq!(r_attr(&e, b"link"), None);
    }

    #[test]
    fn numeric_attrs_return_none_rather_than_zero_on_junk() {
        let e = start(r#"<a:off x="914400" y="oops"/>"#);
        assert_eq!(attr_i64(&e, b"x"), Some(914_400));
        assert_eq!(attr_i64(&e, b"y"), None);
        assert_eq!(attr_i64(&e, b"z"), None);
    }

    #[test]
    fn booleans_accept_every_form_ooxml_uses() {
        let e = start(r#"<a:rPr b="1" i="true" u="0" strike="false" cap=""/>"#);
        assert_eq!(attr_bool(&e, b"b"), Some(true));
        assert_eq!(attr_bool(&e, b"i"), Some(true));
        assert_eq!(attr_bool(&e, b"u"), Some(false));
        assert_eq!(attr_bool(&e, b"strike"), Some(false));
        assert_eq!(attr_bool(&e, b"cap"), Some(true));
    }

    #[test]
    fn percentages_handle_both_the_thousandths_and_the_signed_form() {
        let e = start(r#"<a:alpha val="50000"/>"#);
        assert_eq!(attr_percent(&e, b"val"), Some(0.5));
        let e = start(r#"<a:alpha val="50%"/>"#);
        assert_eq!(attr_percent(&e, b"val"), Some(0.5));
    }

    #[test]
    fn text_content_gathers_across_nested_elements() {
        let xml = b"<a:p><a:r><a:t>Hello </a:t></a:r><a:r><a:t>world</a:t></a:r></a:p>";
        let mut r = Reader::new(xml);
        let mut buf = Vec::new();
        // Enter <a:p>.
        let _ = r.read_event_into(&mut buf);
        assert_eq!(text_content(&mut r, b"p"), "Hello world");
    }

    #[test]
    fn text_content_preserves_significant_whitespace() {
        let xml = b"<a:t>  spaced  </a:t>";
        let mut r = Reader::new(xml);
        let mut buf = Vec::new();
        let _ = r.read_event_into(&mut buf);
        assert_eq!(text_content(&mut r, b"t"), "  spaced  ");
    }

    #[test]
    fn skip_element_lands_after_the_matching_close_tag() {
        let xml = b"<root><skipme><skipme/><x/></skipme><after/></root>";
        let mut r = Reader::new(xml);
        let mut buf = Vec::new();
        let _ = r.read_event_into(&mut buf); // <root>
        let _ = r.read_event_into(&mut buf); // <skipme>
        skip_element(&mut r, b"skipme");
        let mut buf2 = Vec::new();
        match r.read_event_into(&mut buf2) {
            Ok(Event::Empty(e)) => assert_eq!(local_name(e.name().as_ref()), b"after"),
            other => panic!("expected <after/>, got {other:?}"),
        }
    }
}
