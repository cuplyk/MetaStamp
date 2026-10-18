//! Reading and rewriting the core properties part inside an OOXML package.
//!
//! The rewrite copies every ZIP entry other than the core properties part
//! byte for byte (no recompression), keeps each entry's internal ZIP
//! timestamp, and writes the result to a temporary file in the same folder
//! before swapping it in, so a failure never leaves a half-written document.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result};

const RELS_PART: &str = "_rels/.rels";
const NS_PACKAGE_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const REL_CORE_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const DEFAULT_CORE_PART: &str = "docProps/core.xml";

type Archive = ZipArchive<BufReader<File>>;

fn open_archive(path: &Path) -> Result<Archive> {
    Ok(ZipArchive::new(BufReader::new(File::open(path)?))?)
}

fn read_entry(archive: &mut Archive, name: &str) -> Result<Option<String>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| Error::xml(name, e))?;
    Ok(Some(text))
}

/// Finds the core properties part through the package relationships, falling
/// back to the conventional `docProps/core.xml`.
fn core_part_name(archive: &mut Archive) -> Result<Option<String>> {
    if let Some(rels) = read_entry(archive, RELS_PART)?
        && let Some(target) = core_target_from_rels(&rels)?
    {
        let name = target.trim_start_matches('/').to_owned();
        if archive.index_for_name(&name).is_some() {
            return Ok(Some(name));
        }
    }
    Ok(archive
        .index_for_name(DEFAULT_CORE_PART)
        .map(|_| DEFAULT_CORE_PART.to_owned()))
}

fn core_target_from_rels(xml: &str) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(xml);
    loop {
        let (ns, event) = reader
            .read_resolved_event()
            .map_err(|e| Error::xml(RELS_PART, e))?;
        match event {
            Event::Start(rel) | Event::Empty(rel)
                if matches!(ns, ResolveResult::Bound(n) if n.as_ref() == NS_PACKAGE_RELS)
                    && rel.local_name().as_ref() == "Relationship" =>
            {
                let mut rel_type = None;
                let mut target = None;
                for attr in rel.attributes() {
                    let attr = attr.map_err(|e| Error::xml(RELS_PART, e))?;
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| Error::xml(RELS_PART, e))?
                        .into_owned();
                    match attr.key.as_ref() {
                        "Type" => rel_type = Some(value),
                        "Target" => target = Some(value),
                        _ => {}
                    }
                }
                if rel_type.as_deref() == Some(REL_CORE_PROPERTIES) {
                    return Ok(target);
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Reads the XML of the core properties part.
///
/// Fails with [`Error::Zip`] if the file is not a ZIP package and with
/// [`Error::MissingCoreProperties`] if it has no core properties part.
pub fn read_core_xml(path: &Path) -> Result<String> {
    let mut archive = open_archive(path)?;
    let name = core_part_name(&mut archive)?.ok_or(Error::MissingCoreProperties)?;
    read_entry(&mut archive, &name)?.ok_or(Error::MissingCoreProperties)
}

/// Replaces the core properties part with `core_xml`, atomically.
///
/// The swap creates a new file, so the caller is responsible for restoring
/// the filesystem timestamps afterwards (see [`crate::stamp::stamp`]).
pub fn write_core_xml(path: &Path, core_xml: &str) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.permissions().readonly() {
        return Err(Error::ReadOnly(path.to_owned()));
    }

    let tmp = temp_path(path);
    let result = write_package(path, &tmp, core_xml).and_then(|()| {
        fs::set_permissions(&tmp, metadata.permissions())?;
        fs::rename(&tmp, path)?;
        Ok(())
    });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_package(src: &Path, dst: &Path, core_xml: &str) -> Result<()> {
    let mut archive = open_archive(src)?;
    let core_name = core_part_name(&mut archive)?.ok_or(Error::MissingCoreProperties)?;

    let out = File::create_new(dst)?;
    let mut writer = ZipWriter::new(BufWriter::new(out));
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        if entry.name()? == core_name {
            let mut options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            if let Some(time) = entry.last_modified() {
                options = options.last_modified_time(time);
            }
            drop(entry);
            writer.start_file(core_name.as_str(), options)?;
            writer.write_all(core_xml.as_bytes())?;
        } else {
            writer.raw_copy_file(entry)?;
        }
    }
    let file = writer.finish()?.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    Ok(())
}

/// A hidden sibling of `path`, so the final rename never crosses volumes.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.metastamp-{}.tmp", std::process::id()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#;
    pub const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    pub const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:creator>Author</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2021-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2021-01-02T00:00:00Z</dcterms:modified></cp:coreProperties>"#;
    pub const DOCUMENT: &str = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#;

    /// Writes a minimal but valid `.docx` package.
    pub fn write_docx(path: &Path, core: Option<&str>) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap());
        zip.start_file("[Content_Types].xml", deflated).unwrap();
        zip.write_all(CONTENT_TYPES.as_bytes()).unwrap();
        zip.start_file("_rels/.rels", deflated).unwrap();
        zip.write_all(RELS.as_bytes()).unwrap();
        if let Some(core) = core {
            zip.start_file("docProps/core.xml", deflated).unwrap();
            zip.write_all(core.as_bytes()).unwrap();
        }
        zip.start_file("word/document.xml", stored).unwrap();
        zip.write_all(DOCUMENT.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    fn entries(path: &Path) -> Vec<(String, CompressionMethod, Vec<u8>)> {
        let mut archive = open_archive(path).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (
                    entry.name().unwrap().into_owned(),
                    entry.compression(),
                    data,
                )
            })
            .collect()
    }

    #[test]
    fn reads_core_xml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));
        assert_eq!(read_core_xml(&path).unwrap(), CORE);
    }

    #[test]
    fn missing_core_part_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, None);
        assert!(matches!(
            read_core_xml(&path),
            Err(Error::MissingCoreProperties)
        ));
        assert!(matches!(
            write_core_xml(&path, CORE),
            Err(Error::MissingCoreProperties)
        ));
    }

    #[test]
    fn non_zip_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        fs::write(&path, "not a zip").unwrap();
        assert!(matches!(read_core_xml(&path), Err(Error::Zip(_))));
    }

    #[test]
    fn rewrite_changes_only_the_core_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));
        let before = entries(&path);

        let new_core = CORE.replace("Author", "Someone else");
        write_core_xml(&path, &new_core).unwrap();

        let after = entries(&path);
        assert_eq!(before.len(), after.len());
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(b.0, a.0, "entry order");
            assert_eq!(b.1, a.1, "compression of {}", b.0);
            if b.0 == "docProps/core.xml" {
                assert_eq!(a.2, new_core.as_bytes());
            } else {
                assert_eq!(b.2, a.2, "content of {}", b.0);
            }
        }
        assert_eq!(read_core_xml(&path).unwrap(), new_core);
        // No temporary file is left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn read_only_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();

        assert!(matches!(
            write_core_xml(&path, CORE),
            Err(Error::ReadOnly(_))
        ));

        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn finds_core_part_through_relationships() {
        let rels = RELS.replace("Target=\"docProps/core.xml\"", "Target=\"/meta/props.xml\"");
        assert_eq!(
            core_target_from_rels(&rels).unwrap().as_deref(),
            Some("/meta/props.xml")
        );
    }
}
