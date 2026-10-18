# MetaStamp

A lightweight desktop app (Rust + Tauri) to quickly edit the internal metadata
(`docProps/core.xml`) and the filesystem timestamps (created / modified) of
Office documents.

## Layout

| Path | What it is |
|---|---|
| `crates/metastamp-core` | The engine: reads and patches `core.xml`, rewrites the package safely, sets filesystem timestamps with the standard library. No Windows Property System, no Office required. |
| `BulkFileChanger.*` | Reference tool used for the static analysis that shaped the design. |

## Core modules

- `core_props`: parse and patch `docProps/core.xml` (`dcterms:created`, `dcterms:modified`, `cp:lastPrinted`, `dc:creator`, `cp:lastModifiedBy`, `cp:revision`, `dc:title`). Only the edited values change. Missing elements and namespace declarations are added, and custom prefixes are kept.
- `package`: finds the core part through `_rels/.rels`, copies every other ZIP entry byte for byte, keeps internal ZIP timestamps, and swaps in the new file atomically (temp file in the same folder + rename).
- `fs_times`: created / modified / accessed via `File::set_times`. It only touches the requested times and works on folders and read-only files. Creation time can be set on Windows and macOS; Linux has no API for it.
- `timeconv`: local time → UTC using the offset in force on the target date (DST-correct), plus W3CDTF formatting and parsing.
- `stamp`: `inspect()` and `stamp()`. Metadata is written first and filesystem times last, and the original times are restored unless new ones were requested.

```rust
use metastamp_core::{stamp, CoreProperties, StampRequest, TimestampEdits};
use metastamp_core::timeconv::{local_to_utc, to_system_time};

let when = local_to_utc("2021-03-04T10:20:00".parse()?)?;
stamp("report.docx".as_ref(), &StampRequest {
    core: CoreProperties { created: Some(when), modified: Some(when), ..Default::default() },
    times: TimestampEdits {
        created: Some(to_system_time(when)),
        modified: Some(to_system_time(when)),
        ..Default::default()
    },
})?;
```

## Development

```
cargo test
cargo clippy --all-targets -- -D warnings
```

CI runs both on Linux, Windows and macOS.
