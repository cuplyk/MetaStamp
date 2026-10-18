//! Tauri commands. All the logic lives in plain functions so it can be
//! tested without a window; the `#[tauri::command]` wrappers only adapt them.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use metastamp_core::fs_times::CREATION_TIME_SUPPORTED;
use metastamp_core::timeconv::{from_system_time, local_to_utc, to_system_time};
use metastamp_core::{CoreProperties, StampRequest, TimestampEdits, inspect, stamp};
use serde::{Deserialize, Serialize};

/// One file as shown in the list. Times are RFC 3339 in UTC; the interface
/// formats them for display.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
    /// `None` when the file is not an Office Open XML document.
    pub core: Option<CoreView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoreView {
    pub created: Option<String>,
    pub modified: Option<String>,
    pub last_printed: Option<String>,
    pub creator: Option<String>,
    pub last_modified_by: Option<String>,
    pub revision: Option<String>,
    pub title: Option<String>,
}

/// Which time zone the dates typed in the interface are in.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputZone {
    #[default]
    Local,
    Utc,
}

/// Dates are `YYYY-MM-DDTHH:MM[:SS]` as produced by `<input type="datetime-local">`.
/// A `null` field means "leave unchanged".
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct FsEdits {
    pub created: Option<String>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CoreEdits {
    pub created: Option<String>,
    pub modified: Option<String>,
    pub last_printed: Option<String>,
    pub creator: Option<String>,
    pub last_modified_by: Option<String>,
    pub revision: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ApplyRequest {
    pub paths: Vec<String>,
    pub zone: InputZone,
    pub fs: FsEdits,
    pub core: CoreEdits,
    /// Set each document's file created/modified times from its own
    /// `dcterms:created` / `dcterms:modified` (after the edits above).
    pub fs_from_core: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub path: String,
    pub ok: bool,
    pub message: Option<String>,
    pub file: Option<FileView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub creation_time: bool,
}

fn rfc3339(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn view(path: &Path) -> FileView {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut file = FileView {
        path: path.display().to_string(),
        name,
        is_dir: path.is_dir(),
        created: None,
        modified: None,
        accessed: None,
        core: None,
        error: None,
    };
    match inspect(path) {
        Ok(info) => {
            let t = info.times;
            file.created = t.created.map(|c| rfc3339(from_system_time(c)));
            file.modified = Some(rfc3339(from_system_time(t.modified)));
            file.accessed = Some(rfc3339(from_system_time(t.accessed)));
            file.core = info.core.map(|c| CoreView {
                created: c.created.map(rfc3339),
                modified: c.modified.map(rfc3339),
                last_printed: c.last_printed.map(rfc3339),
                creator: c.creator,
                last_modified_by: c.last_modified_by,
                revision: c.revision,
                title: c.title,
            });
        }
        Err(e) => file.error = Some(e.to_string()),
    }
    file
}

/// Parses a date typed in the interface and converts it to UTC.
pub fn parse_input(value: &str, zone: InputZone) -> Result<DateTime<Utc>, String> {
    let value = value.trim();
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))
        .map_err(|_| format!("\"{value}\" is not a valid date and time"))?;
    match zone {
        InputZone::Utc => Ok(naive.and_utc()),
        InputZone::Local => local_to_utc(naive).map_err(|e| e.to_string()),
    }
}

fn parse_opt(value: &Option<String>, zone: InputZone) -> Result<Option<DateTime<Utc>>, String> {
    value.as_deref().map(|v| parse_input(v, zone)).transpose()
}

/// Turns the interface's edits into the request for the core library,
/// shared by every selected file.
pub fn build_request(req: &ApplyRequest) -> Result<StampRequest, String> {
    let z = req.zone;
    let core = CoreProperties {
        created: parse_opt(&req.core.created, z)?,
        modified: parse_opt(&req.core.modified, z)?,
        last_printed: parse_opt(&req.core.last_printed, z)?,
        creator: req.core.creator.clone(),
        last_modified_by: req.core.last_modified_by.clone(),
        revision: req.core.revision.clone(),
        title: req.core.title.clone(),
    };
    let times = TimestampEdits {
        created: parse_opt(&req.fs.created, z)?.map(to_system_time),
        modified: parse_opt(&req.fs.modified, z)?.map(to_system_time),
        accessed: parse_opt(&req.fs.accessed, z)?.map(to_system_time),
    };
    if times.created.is_some() && !CREATION_TIME_SUPPORTED {
        return Err("This system cannot change the file creation date.".into());
    }
    Ok(StampRequest { core, times })
}

/// Applies the request to one file. Document fields are skipped, with a
/// note, for files that are not Office documents.
pub fn apply_one(path: &Path, base: &StampRequest, fs_from_core: bool) -> ApplyResult {
    let mut request = base.clone();
    let mut notes = Vec::new();

    let current = match inspect(path) {
        Ok(info) => info,
        Err(e) => return failure(path, e.to_string()),
    };
    match &current.core {
        None => {
            if !request.core.is_empty() {
                request.core = CoreProperties::default();
                notes.push("document properties skipped: not an Office document");
            }
            if fs_from_core {
                notes.push("file dates not copied: no document properties");
            }
        }
        Some(existing) if fs_from_core => {
            let created = request.core.created.or(existing.created);
            let modified = request.core.modified.or(existing.modified);
            if let Some(created) = created.filter(|_| CREATION_TIME_SUPPORTED) {
                request.times.created = Some(to_system_time(created));
            }
            if let Some(modified) = modified {
                request.times.modified = Some(to_system_time(modified));
            }
        }
        Some(_) => {}
    }

    match stamp(path, &request) {
        Ok(_) => ApplyResult {
            path: path.display().to_string(),
            ok: true,
            message: (!notes.is_empty()).then(|| notes.join("; ")),
            file: Some(view(path)),
        },
        Err(e) => failure(path, e.to_string()),
    }
}

fn failure(path: &Path, message: String) -> ApplyResult {
    ApplyResult {
        path: path.display().to_string(),
        ok: false,
        message: Some(message),
        file: Some(view(path)),
    }
}

pub fn apply_all(req: &ApplyRequest) -> Result<Vec<ApplyResult>, String> {
    let base = build_request(req)?;
    Ok(req
        .paths
        .iter()
        .map(|p| apply_one(&PathBuf::from(p), &base, req.fs_from_core))
        .collect())
}

/// Lists the given paths; folders are expanded one level deep when
/// `expand_folders` is set (their files are added, not the folder itself).
pub fn collect(paths: &[String], expand_folders: bool) -> Vec<FileView> {
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if expand_folders && path.is_dir() {
            let mut children: Vec<PathBuf> = std::fs::read_dir(&path)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file())
                        .collect()
                })
                .unwrap_or_default();
            children.sort();
            out.extend(children.iter().map(|c| view(c)));
        } else {
            out.push(view(&path));
        }
    }
    out
}

#[tauri::command]
pub fn load_files(paths: Vec<String>, expand_folders: bool) -> Vec<FileView> {
    collect(&paths, expand_folders)
}

#[tauri::command]
pub fn apply_changes(request: ApplyRequest) -> Result<Vec<ApplyResult>, String> {
    apply_all(&request)
}

#[tauri::command]
pub fn capabilities() -> Capabilities {
    Capabilities {
        creation_time: CREATION_TIME_SUPPORTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:creator>Author</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2021-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2021-01-02T00:00:00Z</dcterms:modified></cp:coreProperties>"#;

    fn docx(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let opts = SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>",
        )
        .unwrap();
        zip.start_file("docProps/core.xml", opts).unwrap();
        zip.write_all(CORE.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_datetime_local_values() {
        assert_eq!(
            parse_input("2020-02-03T04:05", InputZone::Utc).unwrap(),
            utc("2020-02-03T04:05:00Z")
        );
        assert_eq!(
            parse_input("2020-02-03T04:05:06", InputZone::Utc).unwrap(),
            utc("2020-02-03T04:05:06Z")
        );
        assert!(parse_input("03/02/2020", InputZone::Utc).is_err());
        assert!(parse_input("2020-02-03T04:05", InputZone::Local).is_ok());
    }

    #[test]
    fn request_json_from_the_interface() {
        let json = r#"{
            "paths": ["a.docx"], "zone": "utc",
            "fs": {"modified": "2019-05-05T10:00"},
            "core": {"created": "2019-05-01T09:30", "lastModifiedBy": "Anna"},
            "fsFromCore": false
        }"#;
        let req: ApplyRequest = serde_json::from_str(json).unwrap();
        let stamp = build_request(&req).unwrap();
        assert_eq!(stamp.core.created, Some(utc("2019-05-01T09:30:00Z")));
        assert_eq!(stamp.core.last_modified_by.as_deref(), Some("Anna"));
        assert_eq!(stamp.core.modified, None);
        assert_eq!(
            stamp.times.modified,
            Some(to_system_time(utc("2019-05-05T10:00:00Z")))
        );
        assert_eq!(stamp.times.accessed, None);
    }

    #[test]
    fn applies_to_documents_and_reports_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = docx(dir.path(), "a.docx");
        let req = ApplyRequest {
            paths: vec![path.display().to_string()],
            zone: InputZone::Utc,
            core: CoreEdits {
                modified: Some("2018-08-08T08:08".into()),
                creator: Some("Nuovo autore".into()),
                ..Default::default()
            },
            fs: FsEdits {
                modified: Some("2018-08-09T09:09".into()),
                ..Default::default()
            },
            fs_from_core: false,
        };
        let results = apply_all(&req).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].ok, "{:?}", results[0].message);
        let file = results[0].file.as_ref().unwrap();
        let core = file.core.as_ref().unwrap();
        assert_eq!(core.modified.as_deref(), Some("2018-08-08T08:08:00Z"));
        assert_eq!(core.created.as_deref(), Some("2021-01-01T00:00:00Z"));
        assert_eq!(core.creator.as_deref(), Some("Nuovo autore"));
        assert_eq!(file.modified.as_deref(), Some("2018-08-09T09:09:00Z"));
    }

    #[test]
    fn copies_document_dates_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = docx(dir.path(), "a.docx");
        let req = ApplyRequest {
            paths: vec![path.display().to_string()],
            fs_from_core: true,
            ..Default::default()
        };
        let results = apply_all(&req).unwrap();
        assert!(results[0].ok, "{:?}", results[0].message);
        let file = results[0].file.as_ref().unwrap();
        assert_eq!(file.modified.as_deref(), Some("2021-01-02T00:00:00Z"));
        if CREATION_TIME_SUPPORTED {
            assert_eq!(file.created.as_deref(), Some("2021-01-01T00:00:00Z"));
        }
    }

    #[test]
    fn plain_files_get_file_dates_and_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hi").unwrap();
        let req = ApplyRequest {
            paths: vec![path.display().to_string()],
            zone: InputZone::Utc,
            core: CoreEdits {
                title: Some("x".into()),
                ..Default::default()
            },
            fs: FsEdits {
                modified: Some("2010-10-10T10:10".into()),
                ..Default::default()
            },
            fs_from_core: false,
        };
        let results = apply_all(&req).unwrap();
        assert!(results[0].ok);
        assert!(results[0].message.as_deref().unwrap().contains("skipped"));
        let file = results[0].file.as_ref().unwrap();
        assert_eq!(file.modified.as_deref(), Some("2010-10-10T10:10:00Z"));
        assert!(file.core.is_none());
    }

    #[test]
    fn missing_file_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let ok = docx(dir.path(), "ok.docx");
        let req = ApplyRequest {
            paths: vec![
                dir.path().join("missing.docx").display().to_string(),
                ok.display().to_string(),
            ],
            zone: InputZone::Utc,
            fs: FsEdits {
                modified: Some("2015-01-01T00:00".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let results = apply_all(&req).unwrap();
        assert!(!results[0].ok);
        assert!(results[1].ok);
    }

    #[test]
    fn bad_date_rejects_the_whole_request() {
        let req = ApplyRequest {
            fs: FsEdits {
                modified: Some("tomorrow".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(apply_all(&req).is_err());
    }

    #[test]
    fn folders_expand_to_their_files() {
        let dir = tempfile::tempdir().unwrap();
        docx(dir.path(), "b.docx");
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let root = vec![dir.path().display().to_string()];

        let names: Vec<_> = collect(&root, true).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["a.txt", "b.docx"]);

        let only = collect(&root, false);
        assert_eq!(only.len(), 1);
        assert!(only[0].is_dir);
    }
}
