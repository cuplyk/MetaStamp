//! Filesystem timestamps (created / modified / accessed), standard library only.
//!
//! Mirrors what BulkFileChanger does with `CreateFileW` + `SetFileTime`, using
//! [`std::fs::File::set_times`] instead:
//!
//! * only the times that are requested are changed, the others are left alone;
//! * on Windows the handle asks only for `FILE_WRITE_ATTRIBUTES`, shares
//!   read/write/delete, and uses `FILE_FLAG_BACKUP_SEMANTICS` so folders work too;
//! * the creation time can be set on Windows and macOS. Linux has no API to
//!   change it, so asking for it there fails with
//!   [`crate::Error::CreationTimeUnsupported`].

use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::time::SystemTime;

use crate::error::Result;

/// Whether this platform can set a file's creation time.
pub const CREATION_TIME_SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// The current timestamps of a file or folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTimestamps {
    /// `None` when the filesystem does not record a creation time.
    pub created: Option<SystemTime>,
    pub modified: SystemTime,
    pub accessed: SystemTime,
}

/// Timestamps to set. `None` leaves that timestamp unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimestampEdits {
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
}

impl TimestampEdits {
    pub fn is_empty(&self) -> bool {
        self.created.is_none() && self.modified.is_none() && self.accessed.is_none()
    }
}

/// Reads the timestamps of `path`, following symlinks.
pub fn read(path: &Path) -> Result<FileTimestamps> {
    let metadata = fs::metadata(path)?;
    Ok(FileTimestamps {
        created: metadata.created().ok(),
        modified: metadata.modified()?,
        accessed: metadata.accessed()?,
    })
}

/// Sets the requested timestamps of a file or folder.
pub fn apply(path: &Path, edits: &TimestampEdits) -> Result<()> {
    if edits.is_empty() {
        return Ok(());
    }
    let mut times = FileTimes::new();
    if let Some(modified) = edits.modified {
        times = times.set_modified(modified);
    }
    if let Some(accessed) = edits.accessed {
        times = times.set_accessed(accessed);
    }
    if let Some(created) = edits.created {
        times = set_created(times, created)?;
    }
    open_for_times(path)?.set_times(times)?;
    Ok(())
}

#[cfg(windows)]
fn set_created(times: FileTimes, created: SystemTime) -> Result<FileTimes> {
    use std::os::windows::fs::FileTimesExt;
    Ok(times.set_created(created))
}

#[cfg(target_os = "macos")]
fn set_created(times: FileTimes, created: SystemTime) -> Result<FileTimes> {
    use std::os::macos::fs::FileTimesExt;
    Ok(times.set_created(created))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_created(_times: FileTimes, _created: SystemTime) -> Result<FileTimes> {
    Err(crate::Error::CreationTimeUnsupported)
}

#[cfg(windows)]
fn open_for_times(path: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0001 | 0x0002 | 0x0004;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    Ok(fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?)
}

#[cfg(not(windows))]
fn open_for_times(path: &Path) -> Result<File> {
    // futimens() only needs a descriptor, so a read-only open works for
    // files and folders alike.
    Ok(File::open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use std::time::{Duration, UNIX_EPOCH};

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn sets_modified_and_accessed_independently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, "x").unwrap();

        apply(
            &path,
            &TimestampEdits {
                modified: Some(at(1_000_000_000)),
                accessed: Some(at(1_100_000_000)),
                ..Default::default()
            },
        )
        .unwrap();
        let t = read(&path).unwrap();
        assert_eq!(t.modified, at(1_000_000_000));
        assert_eq!(t.accessed, at(1_100_000_000));

        // Changing only the modified time leaves the accessed time alone.
        apply(
            &path,
            &TimestampEdits {
                modified: Some(at(1_200_000_000)),
                ..Default::default()
            },
        )
        .unwrap();
        let t = read(&path).unwrap();
        assert_eq!(t.modified, at(1_200_000_000));
        assert_eq!(t.accessed, at(1_100_000_000));
    }

    #[test]
    fn works_on_folders() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        apply(
            &sub,
            &TimestampEdits {
                modified: Some(at(946_684_800)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&sub).unwrap().modified, at(946_684_800));
    }

    #[test]
    fn works_on_read_only_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ro.txt");
        fs::write(&path, "x").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();

        apply(
            &path,
            &TimestampEdits {
                modified: Some(at(1_300_000_000)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&path).unwrap().modified, at(1_300_000_000));

        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn creation_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.txt");
        fs::write(&path, "x").unwrap();
        let edits = TimestampEdits {
            created: Some(at(978_307_200)),
            ..Default::default()
        };
        if CREATION_TIME_SUPPORTED {
            apply(&path, &edits).unwrap();
            assert_eq!(read(&path).unwrap().created, Some(at(978_307_200)));
        } else {
            assert!(matches!(
                apply(&path, &edits),
                Err(Error::CreationTimeUnsupported)
            ));
        }
    }

    #[test]
    fn empty_edit_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.txt");
        // Nothing to do, so the missing file is never opened.
        apply(&path, &TimestampEdits::default()).unwrap();
    }
}
