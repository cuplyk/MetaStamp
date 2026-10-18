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

1. **Add files**, **Add folder** or drag files and folders onto the window. Word, Excel and PowerPoint files show a coloured icon.
2. Select the files to change: click, Ctrl/Cmd-click to add one, Shift-click for a range, or Ctrl/Cmd+A for all. The filter box narrows the list, and the column headers sort it.
3. The right-hand panel shows the values the selected files share, or **Mixed values** where they differ. Edit the ones you want to change. Each edited field gets a dot and an undo button, and the clock button inserts the current time.
4. Click **Apply** (Ctrl/Cmd+Enter) to write every edited value to every selected file.

Only edited values change. Editing document properties alone keeps the file's own dates. **Match document dates** sets each document's file created/modified dates to its own document created/last saved dates. Files that are not Office documents get their file dates changed and their document fields skipped, and a note says so.

The **Local time / UTC** switch in the title bar applies to every date, both the ones shown and the ones you type.

Keyboard: Ctrl/Cmd+O adds files, Shift+Ctrl/Cmd+O adds a folder, Ctrl/Cmd+F filters, the arrow keys move the selection (with Shift to extend it), Delete removes files from the list (it never deletes them from disk), and Esc clears the selection.

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

To build installers: `cargo install tauri-cli --version "^2"`, then run `cargo tauri build` in `app/`. The **Build installers** workflow does this on every push and PR and uploads the Windows `.msi`/`.exe`, the portable Windows `.exe` and the macOS `.dmg` as artifacts.

## Releasing

Set the new version in `app/tauri.conf.json` (and `app/Cargo.toml`), merge it to `main`, then push a matching tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The **Release** workflow checks that the tag matches the app version, builds the Windows `.msi`, setup `.exe` and portable `.exe` and the macOS `.dmg` on GitHub's runners (no local MSVC needed), and publishes them as a GitHub Release with generated notes.

The portable `.exe` needs no installation: put it anywhere (a USB stick too) and run it. It uses the WebView2 runtime that comes with Windows 10 and 11.

To rebuild the files of an existing release with the current workflow, run **Actions → Release → Run workflow** and enter its tag.

CI runs fmt, clippy and the tests on Linux, Windows and macOS.
