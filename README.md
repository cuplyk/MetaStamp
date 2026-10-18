# MetaStamp

A lightweight desktop app (Rust + Tauri) to quickly edit the internal metadata
(`docProps/core.xml`) and the filesystem timestamps (created / modified) of
Office documents.

## Layout

| Path | What it is |
|---|---|
| `app` | The desktop app (Tauri 2). `src/commands.rs` exposes the core library to the interface, and `ui/` is plain HTML/CSS/JS with no build step. |
| `crates/metastamp-core` | The engine: reads and patches `core.xml`, rewrites the package safely, sets filesystem timestamps with the standard library. No Windows Property System, no Office required. |
| `BulkFileChanger.*` | Reference tool used for the static analysis that shaped the design. |

## Using the app

1. **Add files…**, **Add folder…** or drag files and folders onto the window. Office documents get a `DOC` badge.
2. Select the rows to change (Ctrl/Cmd-click for several, or the header checkbox for all).
3. In the right-hand panel, tick each value you want to change and enter it. **Fill with values of the first selected file** copies the current values in as a starting point, and **Now** inserts the current time.
4. Choose whether the dates you type are local time or UTC, then click **Apply to selected**.

Only the ticked values change. Editing document properties alone keeps the file's own dates. **Copy each document's created / last saved dates to its file dates** syncs the two per file. Files that are not Office documents get their file dates changed and their document fields skipped, with a note in the status bar.

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
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p metastamp          # start the app
```

On Linux the app needs the WebKitGTK development packages (see `.github/workflows/ci.yml`).

To build installers: `cargo install tauri-cli --version "^2"`, then run `cargo tauri build` in `app/`. The **Build installers** workflow does this on every push and PR and uploads the Windows `.msi`/`.exe` and the macOS `.dmg` as artifacts.

CI runs fmt, clippy and the tests on Linux, Windows and macOS.
