use std::path::PathBuf;

use chrono::NaiveDateTime;

/// Errors returned by MetaStamp core operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid ZIP package: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("invalid XML in {part}: {message}")]
    Xml { part: String, message: String },

    #[error("the package has no core properties part (docProps/core.xml)")]
    MissingCoreProperties,

    #[error("the core properties part does not start with <cp:coreProperties>")]
    NotCoreProperties,

    #[error("{0} is read-only")]
    ReadOnly(PathBuf),

    #[error("setting the creation time is not supported on this platform")]
    CreationTimeUnsupported,

    #[error("{0} does not exist in the local time zone (daylight saving gap)")]
    NonexistentLocalTime(NaiveDateTime),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn xml(part: &str, err: impl std::fmt::Display) -> Self {
        Error::Xml {
            part: part.to_owned(),
            message: err.to_string(),
        }
    }
}
