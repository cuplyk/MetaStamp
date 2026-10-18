//! Core engine for MetaStamp.
//!
//! MetaStamp edits two kinds of timestamps on Office Open XML documents
//! (`.docx`, `.xlsx`, `.pptx`, ...):
//!
//! * the document properties stored inside the package (`docProps/core.xml`),
//! * the filesystem timestamps of the file itself (created / modified / accessed).
//!
//! Everything is done with the Rust standard library plus a ZIP and an XML
//! parser: no Windows Property System, no Office installation required.
//!
//! The order of operations in [`stamp::stamp`] follows the reference analysis
//! of BulkFileChanger: embedded metadata first, filesystem timestamps last,
//! restoring the original filesystem times that the rewrite would otherwise
//! clobber.

pub mod core_props;
pub mod error;
pub mod fs_times;
pub mod package;
pub mod stamp;
pub mod timeconv;

pub use core_props::CoreProperties;
pub use error::{Error, Result};
pub use fs_times::{FileTimestamps, TimestampEdits};
pub use stamp::{DocumentInfo, StampReport, StampRequest, inspect, stamp};
