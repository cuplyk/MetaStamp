//! High-level operations combining the core properties and filesystem times.

use std::path::Path;

use crate::core_props::{self, CoreProperties};
use crate::error::Result;
use crate::fs_times::{self, CREATION_TIME_SUPPORTED, FileTimestamps, TimestampEdits};
use crate::package;

/// What MetaStamp shows for a file before editing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInfo {
    pub times: FileTimestamps,
    /// `None` when the file is not an OOXML package with core properties.
    pub core: Option<CoreProperties>,
}

/// Reads the filesystem timestamps and, when present, the core properties.
pub fn inspect(path: &Path) -> Result<DocumentInfo> {
    let times = fs_times::read(path)?;
    let core = match package::read_core_xml(path) {
        Ok(xml) => Some(core_props::parse(&xml)?),
        Err(crate::Error::Zip(_) | crate::Error::MissingCoreProperties) => None,
        Err(e) => return Err(e),
    };
    Ok(DocumentInfo { times, core })
}

/// A change to apply to one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StampRequest {
    /// Core properties to set inside the package (`None` fields are kept).
    pub core: CoreProperties,
    /// Filesystem timestamps to set (`None` fields are kept).
    pub times: TimestampEdits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampReport {
    pub before: FileTimestamps,
    pub after: FileTimestamps,
    /// Whether `docProps/core.xml` was rewritten.
    pub core_updated: bool,
}

/// Applies `request` to the file at `path`.
///
/// Order of operations, as in the reference analysis:
/// 1. read the current filesystem timestamps;
/// 2. rewrite `docProps/core.xml` if core properties were requested
///    (this replaces the file, which resets its timestamps);
/// 3. set the filesystem timestamps last: the requested ones, and the
///    original values for every timestamp that was not requested, so editing
///    the metadata alone does not change the file's dates.
pub fn stamp(path: &Path, request: &StampRequest) -> Result<StampReport> {
    let before = fs_times::read(path)?;

    let core_updated = !request.core.is_empty();
    if core_updated {
        let xml = package::read_core_xml(path)?;
        let patched = core_props::patch(&xml, &request.core)?;
        package::write_core_xml(path, &patched)?;
    }

    let mut edits = request.times;
    if core_updated {
        edits.modified = edits.modified.or(Some(before.modified));
        edits.accessed = edits.accessed.or(Some(before.accessed));
        if CREATION_TIME_SUPPORTED {
            edits.created = edits.created.or(before.created);
        }
    }
    fs_times::apply(path, &edits)?;

    Ok(StampReport {
        before,
        after: fs_times::read(path)?,
        core_updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::{CORE, write_docx};
    use crate::timeconv::{parse_w3cdtf, to_system_time};
    use std::time::{Duration, UNIX_EPOCH};

    fn at(secs: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn inspect_reads_core_and_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));
        let info = inspect(&path).unwrap();
        let core = info.core.unwrap();
        assert_eq!(core.creator.as_deref(), Some("Author"));
        assert_eq!(core.created, parse_w3cdtf("2021-01-01T00:00:00Z"));
    }

    #[test]
    fn inspect_handles_plain_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hi").unwrap();
        assert_eq!(inspect(&path).unwrap().core, None);
    }

    #[test]
    fn core_edit_keeps_original_filesystem_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));
        fs_times::apply(
            &path,
            &TimestampEdits {
                modified: Some(at(1_500_000_000)),
                accessed: Some(at(1_500_000_100)),
                ..Default::default()
            },
        )
        .unwrap();

        let created = parse_w3cdtf("2018-02-03T04:05:06Z").unwrap();
        let report = stamp(
            &path,
            &StampRequest {
                core: CoreProperties {
                    created: Some(created),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.core_updated);
        assert_eq!(report.after.modified, at(1_500_000_000));
        assert_eq!(report.after.accessed, at(1_500_000_100));
        if CREATION_TIME_SUPPORTED {
            assert_eq!(report.after.created, report.before.created);
        }
        assert_eq!(inspect(&path).unwrap().core.unwrap().created, Some(created));
    }

    #[test]
    fn core_and_filesystem_times_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.docx");
        write_docx(&path, Some(CORE));

        let modified = parse_w3cdtf("2017-07-07T07:07:07Z").unwrap();
        let mut times = TimestampEdits {
            modified: Some(to_system_time(modified)),
            ..Default::default()
        };
        if CREATION_TIME_SUPPORTED {
            times.created = Some(to_system_time(
                parse_w3cdtf("2017-01-01T00:00:00Z").unwrap(),
            ));
        }
        let report = stamp(
            &path,
            &StampRequest {
                core: CoreProperties {
                    modified: Some(modified),
                    last_modified_by: Some("MetaStamp".into()),
                    ..Default::default()
                },
                times,
            },
        )
        .unwrap();

        assert_eq!(report.after.modified, to_system_time(modified));
        if let Some(created) = times.created {
            assert_eq!(report.after.created, Some(created));
        }
        let core = inspect(&path).unwrap().core.unwrap();
        assert_eq!(core.modified, Some(modified));
        assert_eq!(core.last_modified_by.as_deref(), Some("MetaStamp"));
        assert_eq!(core.creator.as_deref(), Some("Author"));
    }

    #[test]
    fn times_only_on_any_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hi").unwrap();
        let report = stamp(
            &path,
            &StampRequest {
                times: TimestampEdits {
                    modified: Some(at(1_234_567_890)),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!report.core_updated);
        assert_eq!(report.after.modified, at(1_234_567_890));
    }

    #[test]
    fn core_edit_on_non_package_fails_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hi").unwrap();
        let result = stamp(
            &path,
            &StampRequest {
                core: CoreProperties {
                    title: Some("x".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(crate::Error::Zip(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
    }
}
