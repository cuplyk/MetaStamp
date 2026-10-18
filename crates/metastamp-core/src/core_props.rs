//! Reading and patching the OOXML core properties part (`docProps/core.xml`).
//!
//! Patching is a streaming rewrite: every event of the original XML is copied
//! through unchanged except the text of the properties being set. Elements that
//! do not exist yet are appended before `</cp:coreProperties>`. Namespace
//! prefixes are taken from the document itself, so a file that uses
//! non-standard prefixes keeps them.

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Writer};

use crate::error::{Error, Result};
use crate::timeconv::{parse_w3cdtf, to_w3cdtf};

pub const NS_CP: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const NS_DCTERMS: &str = "http://purl.org/dc/terms/";
pub const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

const PART: &str = "docProps/core.xml";

/// The properties MetaStamp reads and writes.
///
/// When used as a patch, `Some` means "set this value" and `None` means
/// "leave it as it is".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreProperties {
    /// `dcterms:created`: shown by Office as "Content created".
    pub created: Option<DateTime<Utc>>,
    /// `dcterms:modified`: shown by Office as "Date last saved".
    pub modified: Option<DateTime<Utc>>,
    /// `cp:lastPrinted`.
    pub last_printed: Option<DateTime<Utc>>,
    /// `dc:creator`: the author.
    pub creator: Option<String>,
    /// `cp:lastModifiedBy`.
    pub last_modified_by: Option<String>,
    /// `cp:revision`.
    pub revision: Option<String>,
    /// `dc:title`.
    pub title: Option<String>,
}

impl CoreProperties {
    pub fn is_empty(&self) -> bool {
        *self == CoreProperties::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Created,
    Modified,
    LastPrinted,
    Creator,
    LastModifiedBy,
    Revision,
    Title,
}

impl Field {
    const ALL: [Field; 7] = [
        Field::Created,
        Field::Modified,
        Field::LastPrinted,
        Field::Creator,
        Field::LastModifiedBy,
        Field::Revision,
        Field::Title,
    ];

    fn namespace(self) -> &'static str {
        match self {
            Field::Created | Field::Modified => NS_DCTERMS,
            Field::Creator | Field::Title => NS_DC,
            Field::LastPrinted | Field::LastModifiedBy | Field::Revision => NS_CP,
        }
    }

    fn local_name(self) -> &'static str {
        match self {
            Field::Created => "created",
            Field::Modified => "modified",
            Field::LastPrinted => "lastPrinted",
            Field::Creator => "creator",
            Field::LastModifiedBy => "lastModifiedBy",
            Field::Revision => "revision",
            Field::Title => "title",
        }
    }

    /// `dcterms:created` and `dcterms:modified` must carry
    /// `xsi:type="dcterms:W3CDTF"`, or Office ignores them.
    fn needs_w3cdtf_type(self) -> bool {
        matches!(self, Field::Created | Field::Modified)
    }

    fn matches(self, ns: &ResolveResult<'_>, local: &str) -> bool {
        matches!(ns, ResolveResult::Bound(n) if n.as_ref() == self.namespace())
            && local == self.local_name()
    }

    fn from_name(ns: &ResolveResult<'_>, local: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.matches(ns, local))
    }

    fn get(self, props: &CoreProperties) -> Option<String> {
        let date = |d: &Option<DateTime<Utc>>| d.as_ref().map(to_w3cdtf);
        match self {
            Field::Created => date(&props.created),
            Field::Modified => date(&props.modified),
            Field::LastPrinted => date(&props.last_printed),
            Field::Creator => props.creator.clone(),
            Field::LastModifiedBy => props.last_modified_by.clone(),
            Field::Revision => props.revision.clone(),
            Field::Title => props.title.clone(),
        }
    }

    fn set(self, props: &mut CoreProperties, value: String) {
        match self {
            Field::Created => props.created = parse_w3cdtf(&value),
            Field::Modified => props.modified = parse_w3cdtf(&value),
            Field::LastPrinted => props.last_printed = parse_w3cdtf(&value),
            Field::Creator => props.creator = Some(value),
            Field::LastModifiedBy => props.last_modified_by = Some(value),
            Field::Revision => props.revision = Some(value),
            Field::Title => props.title = Some(value),
        }
    }
}

/// Preferred prefix for each namespace, used only when the document has not
/// declared one.
fn default_prefix(ns: &str) -> &'static str {
    match ns {
        NS_CP => "cp",
        NS_DC => "dc",
        NS_DCTERMS => "dcterms",
        _ => "xsi",
    }
}

/// Reads the core properties from the XML of a `core.xml` part.
pub fn parse(xml: &str) -> Result<CoreProperties> {
    let mut reader = NsReader::from_str(xml);
    let mut props = CoreProperties::default();
    let mut current: Option<(Field, String)> = None;
    let mut depth = 0usize;

    loop {
        let (ns, event) = reader
            .read_resolved_event()
            .map_err(|e| Error::xml(PART, e))?;
        match event {
            Event::Start(start) => {
                if depth == 0 {
                    check_root(&ns, start.local_name().as_ref())?;
                } else if depth == 1 {
                    current = Field::from_name(&ns, start.local_name().as_ref())
                        .map(|f| (f, String::new()));
                }
                depth += 1;
            }
            Event::Empty(start) => {
                if depth == 0 {
                    check_root(&ns, start.local_name().as_ref())?;
                }
            }
            Event::Text(text) => {
                if let Some((_, value)) = current.as_mut() {
                    value.push_str(&text.xml10_content());
                }
            }
            Event::CData(data) => {
                if let Some((_, value)) = current.as_mut() {
                    value.push_str(&data.xml10_content());
                }
            }
            Event::GeneralRef(entity) => {
                if let Some((_, value)) = current.as_mut() {
                    value.push_str(&resolve_entity(&entity).map_err(|e| Error::xml(PART, e))?);
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1
                    && let Some((field, value)) = current.take()
                {
                    field.set(&mut props, value.trim().to_owned());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(Error::xml(PART, "unexpected end of document"));
    }
    Ok(props)
}

fn check_root(ns: &ResolveResult<'_>, local: &str) -> Result<()> {
    let is_root =
        matches!(ns, ResolveResult::Bound(n) if n.as_ref() == NS_CP) && local == "coreProperties";
    if is_root {
        Ok(())
    } else {
        Err(Error::NotCoreProperties)
    }
}

fn resolve_entity(entity: &quick_xml::events::BytesRef<'_>) -> std::result::Result<String, String> {
    if let Some(ch) = entity.resolve_char_ref().map_err(|e| e.to_string())? {
        return Ok(ch.to_string());
    }
    let name: &str = entity;
    let resolved = match name {
        "lt" => "<",
        "gt" => ">",
        "amp" => "&",
        "apos" => "'",
        "quot" => "\"",
        other => return Err(format!("unknown entity &{other};")),
    };
    Ok(resolved.to_owned())
}

/// Returns `xml` with the properties set in `patch` replaced or added.
///
/// Properties that are `None` in `patch`, and every other element, comment
/// and attribute, are copied unchanged.
pub fn patch(xml: &str, patch: &CoreProperties) -> Result<String> {
    let edits: Vec<(Field, String)> = Field::ALL
        .into_iter()
        .filter_map(|f| f.get(patch).map(|v| (f, v)))
        .collect();
    if edits.is_empty() {
        return Ok(xml.to_owned());
    }

    let mut reader = NsReader::from_str(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len() + 256));
    let mut prefixes = Prefixes::default();
    let mut done = vec![false; edits.len()];
    let mut depth = 0usize;
    let write_err = |e: std::io::Error| Error::xml(PART, e);

    loop {
        let (ns, event) = reader
            .read_resolved_event()
            .map_err(|e| Error::xml(PART, e))?;
        match event {
            Event::Start(start) if depth == 0 => {
                check_root(&ns, start.local_name().as_ref())?;
                let root = prefixes.declare_missing(start, &edits)?;
                writer.write_event(Event::Start(root)).map_err(write_err)?;
                depth = 1;
            }
            Event::Empty(start) if depth == 0 => {
                // `<cp:coreProperties/>`: expand it so children can be added.
                check_root(&ns, start.local_name().as_ref())?;
                let root = prefixes.declare_missing(start, &edits)?;
                let end = root.to_end().into_owned();
                writer.write_event(Event::Start(root)).map_err(write_err)?;
                write_missing(&mut writer, &prefixes, &edits, &done)?;
                writer.write_event(Event::End(end)).map_err(write_err)?;
            }
            Event::Start(start) if depth == 1 => {
                let edit = Field::from_name(&ns, start.local_name().as_ref())
                    .and_then(|f| edits.iter().position(|(e, _)| *e == f));
                match edit {
                    Some(i) => {
                        let end = start.to_end().into_owned();
                        skip_element(&mut reader, end.name().as_ref())?;
                        write_value(&mut writer, &prefixes, start, end, &edits[i])?;
                        done[i] = true;
                    }
                    None => {
                        writer.write_event(Event::Start(start)).map_err(write_err)?;
                        depth += 1;
                    }
                }
            }
            Event::Empty(start) if depth == 1 => {
                let edit = Field::from_name(&ns, start.local_name().as_ref())
                    .and_then(|f| edits.iter().position(|(e, _)| *e == f));
                match edit {
                    Some(i) => {
                        let end = start.to_end().into_owned();
                        write_value(&mut writer, &prefixes, start, end, &edits[i])?;
                        done[i] = true;
                    }
                    None => writer.write_event(Event::Empty(start)).map_err(write_err)?,
                }
            }
            Event::Start(start) => {
                writer.write_event(Event::Start(start)).map_err(write_err)?;
                depth += 1;
            }
            Event::End(end) => {
                if depth == 1 {
                    write_missing(&mut writer, &prefixes, &edits, &done)?;
                }
                writer.write_event(Event::End(end)).map_err(write_err)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => writer.write_event(other).map_err(write_err)?,
        }
    }
    if depth != 0 {
        return Err(Error::xml(PART, "unexpected end of document"));
    }
    String::from_utf8(writer.into_inner()).map_err(|e| Error::xml(PART, e))
}

/// Namespace prefixes in scope on the root element.
#[derive(Default)]
struct Prefixes {
    known: Vec<(String, String)>, // (namespace, prefix)
}

impl Prefixes {
    fn prefix(&self, ns: &str) -> &str {
        self.known
            .iter()
            .find(|(n, _)| n == ns)
            .map(|(_, p)| p.as_str())
            .unwrap_or_else(|| default_prefix(ns))
    }

    /// Records the root's `xmlns:*` declarations and adds the ones the edits
    /// need but the document lacks.
    fn declare_missing<'a>(
        &mut self,
        mut root: BytesStart<'a>,
        edits: &[(Field, String)],
    ) -> Result<BytesStart<'a>> {
        for attr in root.attributes() {
            let attr = attr.map_err(|e| Error::xml(PART, e))?;
            let key: &str = attr.key.as_ref();
            if let Some(prefix) = key.strip_prefix("xmlns:") {
                let ns = attr
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| Error::xml(PART, e))?;
                let prefix = prefix.to_owned();
                self.known.push((ns.into_owned(), prefix));
            }
        }

        let mut needed: Vec<&str> = edits.iter().map(|(f, _)| f.namespace()).collect();
        if edits.iter().any(|(f, _)| f.needs_w3cdtf_type()) {
            needed.push(NS_XSI);
        }
        for ns in needed {
            if !self.known.iter().any(|(n, _)| n == ns) {
                let prefix = default_prefix(ns).to_owned();
                root.push_attribute((format!("xmlns:{prefix}").as_str(), ns));
                self.known.push((ns.to_owned(), prefix));
            }
        }
        Ok(root)
    }
}

/// Consumes events up to and including the end tag matching an already-read
/// start tag.
fn skip_element(reader: &mut NsReader<&[u8]>, name: &str) -> Result<()> {
    let mut nested = 0usize;
    loop {
        match reader.read_event().map_err(|e| Error::xml(PART, e))? {
            Event::Start(s) if s.name().as_ref() == name => nested += 1,
            Event::End(e) if e.name().as_ref() == name => {
                if nested == 0 {
                    return Ok(());
                }
                nested -= 1;
            }
            Event::Eof => return Err(Error::xml(PART, "unexpected end of document")),
            _ => {}
        }
    }
}

/// Writes `<start>value</end>`, keeping the original attributes and adding
/// `xsi:type="dcterms:W3CDTF"` where Office requires it.
fn write_value(
    writer: &mut Writer<Vec<u8>>,
    prefixes: &Prefixes,
    mut start: BytesStart<'_>,
    end: BytesEnd<'_>,
    (field, value): &(Field, String),
) -> Result<()> {
    if field.needs_w3cdtf_type() {
        let xsi_type = format!("{}:type", prefixes.prefix(NS_XSI));
        let has_type = start
            .attributes()
            .flatten()
            .any(|a| a.key.as_ref() == xsi_type.as_str());
        if !has_type {
            let type_value = format!("{}:W3CDTF", prefixes.prefix(NS_DCTERMS));
            start.push_attribute((xsi_type.as_str(), type_value.as_str()));
        }
    }
    let err = |e: std::io::Error| Error::xml(PART, e);
    writer.write_event(Event::Start(start)).map_err(err)?;
    writer
        .write_event(Event::Text(BytesText::new(value)))
        .map_err(err)?;
    writer.write_event(Event::End(end)).map_err(err)?;
    Ok(())
}

/// Appends the edited properties that were not present in the document.
fn write_missing(
    writer: &mut Writer<Vec<u8>>,
    prefixes: &Prefixes,
    edits: &[(Field, String)],
    done: &[bool],
) -> Result<()> {
    for ((field, value), _) in edits.iter().zip(done).filter(|(_, d)| !**d) {
        let name = format!(
            "{}:{}",
            prefixes.prefix(field.namespace()),
            field.local_name()
        );
        let start = BytesStart::new(Cow::Owned(name.clone()));
        let end = BytesEnd::new(Cow::Owned(name));
        write_value(writer, prefixes, start, end, &(*field, value.clone()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORD_CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title></dc:title><dc:subject></dc:subject><dc:creator>Mario Rossi &amp; Co</dc:creator><cp:keywords></cp:keywords><dc:description></dc:description><cp:lastModifiedBy>Mario Rossi</cp:lastModifiedBy><cp:revision>3</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">2021-03-04T10:20:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2021-03-05T11:30:00Z</dcterms:modified></cp:coreProperties>"#;

    fn date(s: &str) -> DateTime<Utc> {
        parse_w3cdtf(s).unwrap()
    }

    #[test]
    fn parses_word_core_properties() {
        let props = parse(WORD_CORE).unwrap();
        assert_eq!(props.created, Some(date("2021-03-04T10:20:00Z")));
        assert_eq!(props.modified, Some(date("2021-03-05T11:30:00Z")));
        assert_eq!(props.creator.as_deref(), Some("Mario Rossi & Co"));
        assert_eq!(props.last_modified_by.as_deref(), Some("Mario Rossi"));
        assert_eq!(props.revision.as_deref(), Some("3"));
        assert_eq!(props.title.as_deref(), Some(""));
        assert_eq!(props.last_printed, None);
    }

    #[test]
    fn patch_replaces_only_the_requested_values() {
        let edits = CoreProperties {
            created: Some(date("2019-01-02T03:04:05Z")),
            modified: Some(date("2019-06-07T08:09:10Z")),
            ..Default::default()
        };
        let out = patch(WORD_CORE, &edits).unwrap();
        let expected = WORD_CORE
            .replace("2021-03-04T10:20:00Z", "2019-01-02T03:04:05Z")
            .replace("2021-03-05T11:30:00Z", "2019-06-07T08:09:10Z");
        assert_eq!(out, expected);
    }

    #[test]
    fn patch_escapes_text() {
        let edits = CoreProperties {
            last_modified_by: Some("A <B> & \"C\"".into()),
            ..Default::default()
        };
        let out = patch(WORD_CORE, &edits).unwrap();
        assert_eq!(
            parse(&out).unwrap().last_modified_by.as_deref(),
            Some("A <B> & \"C\"")
        );
    }

    #[test]
    fn patch_adds_missing_elements_and_namespaces() {
        let xml = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"><cp:revision>1</cp:revision></cp:coreProperties>"#;
        let edits = CoreProperties {
            created: Some(date("2020-01-01T00:00:00Z")),
            creator: Some("Anna".into()),
            ..Default::default()
        };
        let out = patch(xml, &edits).unwrap();
        assert!(out.contains(r#"xmlns:dcterms="http://purl.org/dc/terms/""#));
        assert!(out.contains(r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#));
        assert!(out.contains(
            r#"<dcterms:created xsi:type="dcterms:W3CDTF">2020-01-01T00:00:00Z</dcterms:created>"#
        ));
        let props = parse(&out).unwrap();
        assert_eq!(props.created, Some(date("2020-01-01T00:00:00Z")));
        assert_eq!(props.creator.as_deref(), Some("Anna"));
        assert_eq!(props.revision.as_deref(), Some("1"));
    }

    #[test]
    fn patch_keeps_custom_prefixes() {
        let xml = r#"<p:coreProperties xmlns:p="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:t="http://purl.org/dc/terms/" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><t:modified i:type="t:W3CDTF">2000-01-01T00:00:00Z</t:modified></p:coreProperties>"#;
        let edits = CoreProperties {
            created: Some(date("2010-10-10T10:10:10Z")),
            modified: Some(date("2011-11-11T11:11:11Z")),
            ..Default::default()
        };
        let out = patch(xml, &edits).unwrap();
        assert_eq!(
            out,
            r#"<p:coreProperties xmlns:p="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:t="http://purl.org/dc/terms/" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><t:modified i:type="t:W3CDTF">2011-11-11T11:11:11Z</t:modified><t:created i:type="t:W3CDTF">2010-10-10T10:10:10Z</t:created></p:coreProperties>"#
        );
    }

    #[test]
    fn patch_fills_empty_elements_and_adds_type() {
        let xml = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dcterms:modified/></cp:coreProperties>"#;
        let edits = CoreProperties {
            modified: Some(date("2012-12-12T12:12:12Z")),
            ..Default::default()
        };
        let out = patch(xml, &edits).unwrap();
        assert!(out.contains(
            r#"<dcterms:modified xsi:type="dcterms:W3CDTF">2012-12-12T12:12:12Z</dcterms:modified>"#
        ));
    }

    #[test]
    fn empty_patch_is_identity() {
        assert_eq!(
            patch(WORD_CORE, &CoreProperties::default()).unwrap(),
            WORD_CORE
        );
    }

    #[test]
    fn rejects_other_documents() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#;
        assert!(matches!(parse(xml), Err(Error::NotCoreProperties)));
        let edits = CoreProperties {
            title: Some("x".into()),
            ..Default::default()
        };
        assert!(matches!(patch(xml, &edits), Err(Error::NotCoreProperties)));
    }
}
