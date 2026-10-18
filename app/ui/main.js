// MetaStamp interface. Talks to the Rust side through three commands:
// load_files, apply_changes and capabilities.
//
// Editing model: the inspector shows the values the selected files share
// ("Mixed values" where they differ). Editing a field records a pending
// change; Apply writes every pending change to every selected file.
const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;

const IS_MAC = /Mac/.test(navigator.platform);
const MOD = IS_MAC ? "⌘" : "Ctrl+";

const state = {
  files: [], // FileView objects from Rust, in the order they were added
  visible: [], // files after filter and sort, as shown
  selected: new Set(), // paths
  anchor: null, // path where a Shift range starts
  cursor: null, // path moved by the arrow keys
  filter: "",
  sort: { key: null, dir: 1 },
  zone: loadZone(),
  creationTime: true,
  pending: new Map(), // "group.key" -> input value
  baselines: new Map(), // "group.key" -> { value, mixed }
  results: new Map(), // path -> "ok" | "error", from the last apply
  busy: false,
};

const $ = (id) => document.getElementById(id);
const rows = $("rows");

function loadZone() {
  try {
    return localStorage.getItem("zone") === "utc" ? "utc" : "local";
  } catch {
    return "local";
  }
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function icon(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// ---------- dates ----------

function formatParts(iso) {
  const d = new Date(iso);
  const tz = state.zone === "utc" ? { timeZone: "UTC" } : {};
  return [
    d.toLocaleDateString(undefined, { dateStyle: "medium", ...tz }),
    d.toLocaleTimeString(undefined, { timeStyle: "medium", ...tz }),
  ];
}

/** ISO instant -> value for <input type="datetime-local"> in the given zone. */
function toInputValue(iso, zone) {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n) => String(n).padStart(2, "0");
  const parts =
    zone === "utc"
      ? [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds()]
      : [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds()];
  const [y, mo, da, h, mi, s] = parts;
  return `${y}-${pad(mo)}-${pad(da)}T${pad(h)}:${pad(mi)}:${pad(s)}`;
}

/** datetime-local value typed in `zone` -> ISO instant. */
function fromInputValue(value, zone) {
  return new Date(zone === "utc" ? `${value}Z` : value).toISOString();
}

// ---------- files ----------

const DOC_KINDS = {
  docx: "word", docm: "word", dotx: "word", dotm: "word",
  xlsx: "excel", xlsm: "excel", xltx: "excel", xltm: "excel",
  pptx: "powerpoint", pptm: "powerpoint", potx: "powerpoint", potm: "powerpoint", ppsx: "powerpoint", ppsm: "powerpoint",
};
const KIND_LETTER = { word: "W", excel: "X", powerpoint: "P" };

function extension(name) {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

function fileIcon(f) {
  const kind = DOC_KINDS[extension(f.name)] ?? "other";
  const span = el("span", "ficon");
  span.dataset.kind = f.core && kind === "other" ? "word" : kind;
  span.textContent = KIND_LETTER[span.dataset.kind] ?? (extension(f.name).slice(0, 4).toUpperCase() || "FILE");
  return span;
}

function parentOf(path) {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}

function selectedFiles() {
  return state.files.filter((f) => state.selected.has(f.path));
}

const SORT_VALUE = {
  name: (f) => f.name.toLowerCase(),
  modified: (f) => f.modified,
  created: (f) => f.created,
  docCreated: (f) => f.core?.created,
  docModified: (f) => f.core?.modified,
};

function computeVisible() {
  const q = state.filter.trim().toLowerCase();
  const list = q ? state.files.filter((f) => f.path.toLowerCase().includes(q)) : [...state.files];
  const { key, dir } = state.sort;
  if (key) {
    const value = SORT_VALUE[key];
    list.sort((a, b) => {
      const x = value(a) ?? null;
      const y = value(b) ?? null;
      if (x === y) return 0;
      if (x === null) return 1; // blanks last in both directions
      if (y === null) return -1;
      return dir * (x < y ? -1 : 1);
    });
  }
  state.visible = list;
}

// ---------- file list ----------

function dateCell(iso) {
  const td = el("td");
  if (!iso) {
    td.append(el("span", "none", "—"));
    return td;
  }
  const [date, time] = formatParts(iso);
  const box = el("div", "date");
  box.append(el("span", null, date), el("span", null, time));
  td.append(box);
  return td;
}

function rowFor(f) {
  const tr = el("tr");
  tr.dataset.path = f.path;
  if (state.selected.has(f.path)) tr.classList.add("selected");
  if (f.error) tr.classList.add("error");

  const check = el("td", "c-check");
  const box = el("input", "cb");
  box.type = "checkbox";
  box.tabIndex = -1;
  box.checked = state.selected.has(f.path);
  check.append(box);

  const name = el("td");
  name.title = f.error ? `${f.path}\n\n${f.error}` : f.path;
  const cell = el("div", "name-cell");
  const text = el("div", "name-text");
  const sub = el("span", "fpath");
  sub.append(el("bdi", null, f.error || parentOf(f.path)));
  text.append(el("span", "fname", f.name), sub);
  cell.append(fileIcon(f), text);
  const result = state.results.get(f.path);
  if (result) {
    const mark = el("span", `row-state ${result}`);
    mark.append(icon(result === "ok" ? "check" : "alert"));
    cell.append(mark);
  }
  name.append(cell);

  tr.append(check, name, dateCell(f.modified), dateCell(f.created), dateCell(f.core?.created), dateCell(f.core?.modified));
  return tr;
}

function renderList() {
  computeVisible();
  rows.replaceChildren(...state.visible.map(rowFor));

  const total = state.files.length;
  $("empty").classList.toggle("hidden", total > 0);
  $("no-match").classList.toggle("hidden", total === 0 || state.visible.length > 0);

  const title = $("list-title");
  title.replaceChildren("Files");
  if (total > 0) {
    const shown = state.visible.length === total ? `${total}` : `${state.visible.length} of ${total}`;
    title.append(el("small", null, shown));
  }

  const n = state.selected.size;
  const all = $("select-all");
  all.checked = n > 0 && n === state.visible.length;
  all.indeterminate = n > 0 && n < state.visible.length;
  $("remove").disabled = n === 0;
  $("clear").disabled = total === 0;

  for (const th of document.querySelectorAll("th[data-sort]")) {
    const on = th.dataset.sort === state.sort.key;
    th.classList.toggle("sorted", on);
    th.classList.toggle("desc", on && state.sort.dir < 0);
  }

  const docs = state.files.filter((f) => f.core).length;
  $("counts").textContent = total === 0 ? "No files" : `${plural(total, "file")} · ${plural(docs, "Office document")}`;
}

function render() {
  renderList();
  renderInspector();
}

async function addPaths(paths, expandFolders) {
  if (!paths || paths.length === 0) return;
  let views;
  try {
    views = await invoke("load_files", { paths, expandFolders });
  } catch (e) {
    toast("error", "Couldn't read the files", String(e));
    return;
  }
  const known = new Set(state.files.map((f) => f.path));
  const added = views.filter((v) => !known.has(v.path));
  state.files.push(...added);
  if (added.length > 0) {
    // Select what was just added, so it can be edited straight away.
    state.selected = new Set(added.map((v) => v.path));
    state.anchor = state.cursor = added[0].path;
  }
  recomputeBaselines();
  render();
  if (added.length === 0) toast("warn", "Nothing new to add", "Those files are already in the list.");
}

async function addFiles() {
  const picked = await dialog.open({
    multiple: true,
    filters: [
      { name: "Office documents", extensions: Object.keys(DOC_KINDS) },
      { name: "All files", extensions: ["*"] },
    ],
  });
  if (picked) await addPaths([].concat(picked), false);
}

async function addFolder() {
  const picked = await dialog.open({ directory: true, multiple: true });
  if (picked) await addPaths([].concat(picked), true);
}

function removeSelected() {
  if (state.selected.size === 0) return;
  state.files = state.files.filter((f) => !state.selected.has(f.path));
  for (const p of state.selected) state.results.delete(p);
  setSelection([]);
}

// ---------- selection ----------

function setSelection(paths, anchor = null) {
  state.selected = new Set(paths);
  if (anchor) state.anchor = state.cursor = anchor;
  recomputeBaselines();
  render();
}

function rangeBetween(from, to) {
  const paths = state.visible.map((f) => f.path);
  let i = paths.indexOf(from);
  const j = paths.indexOf(to);
  if (i < 0) i = j;
  return paths.slice(Math.min(i, j), Math.max(i, j) + 1);
}

rows.addEventListener("click", (e) => {
  const tr = e.target.closest("tr");
  if (!tr) return;
  const path = tr.dataset.path;
  const toggle = e.target.closest(".c-check") || e.ctrlKey || e.metaKey;
  if (e.shiftKey && state.anchor) {
    const range = rangeBetween(state.anchor, path);
    state.cursor = path;
    setSelection(toggle ? [...state.selected, ...range] : range);
  } else if (toggle) {
    const next = new Set(state.selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelection(next, path);
  } else {
    setSelection([path], path);
  }
});

$("select-all").addEventListener("change", (e) => {
  setSelection(e.target.checked ? state.visible.map((f) => f.path) : []);
});

function moveCursor(step, extend) {
  if (state.visible.length === 0) return;
  const paths = state.visible.map((f) => f.path);
  const at = paths.indexOf(state.cursor);
  const next = paths[Math.min(Math.max(at < 0 ? 0 : at + step, 0), paths.length - 1)];
  if (extend && state.anchor) {
    state.cursor = next;
    setSelection(rangeBetween(state.anchor, next));
  } else {
    setSelection([next], next);
  }
  rows.querySelector(`tr[data-path="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" });
}

// ---------- filter and sort ----------

$("filter").addEventListener("input", (e) => {
  state.filter = e.target.value;
  computeVisible();
  // Keep the selection to what can be seen, so Apply never touches hidden files.
  const visible = new Set(state.visible.map((f) => f.path));
  setSelection([...state.selected].filter((p) => visible.has(p)));
});

for (const th of document.querySelectorAll("th[data-sort]")) {
  th.append(icon("sort"));
  th.addEventListener("click", () => {
    const key = th.dataset.sort;
    state.sort = { key, dir: state.sort.key === key ? -state.sort.dir : 1 };
    renderList();
  });
}

// ---------- inspector ----------

const fields = [...document.querySelectorAll(".field")].map((node) => ({
  node,
  group: node.dataset.group,
  key: node.dataset.key,
  id: `${node.dataset.group}.${node.dataset.key}`,
  input: node.querySelector("input"),
  date: node.dataset.type === "date",
}));

/** "Match document dates" is on and has documents to take dates from. */
function matching() {
  return $("fs-from-core").checked && selectedFiles().some((s) => s.core);
}

/** File created/modified are taken from each document while matching. */
function lockedByMatch(f) {
  return f.group === "fs" && (f.key === "created" || f.key === "modified") && matching();
}

function unsupported(f) {
  return f.id === "fs.created" && !state.creationTime;
}

function recomputeBaselines() {
  const sel = selectedFiles();
  const docs = sel.filter((s) => s.core);
  for (const f of fields) {
    const values = f.group === "fs" ? sel.map((s) => s[f.key] ?? null) : docs.map((s) => s.core[f.key] ?? null);
    const first = values[0] ?? null;
    const mixed = values.some((v) => v !== first);
    const value = mixed || first === null ? "" : f.date ? toInputValue(first, state.zone) : first;
    state.baselines.set(f.id, { value, mixed });
  }
}

/** The pending changes Apply would send, as [field, value]. */
function effectiveChanges() {
  const sel = selectedFiles();
  const hasDocs = sel.some((s) => s.core);
  return fields
    .filter((f) => state.pending.has(f.id) && !lockedByMatch(f) && !unsupported(f))
    .filter((f) => f.group === "fs" || hasDocs)
    .map((f) => [f, state.pending.get(f.id)]);
}

function renderInspector() {
  const sel = selectedFiles();
  const docs = sel.filter((s) => s.core).length;
  const others = sel.length - docs;

  // Header
  const iconBox = $("sel-icon");
  iconBox.replaceChildren();
  if (sel.length === 1) {
    iconBox.append(fileIcon(sel[0]));
    $("sel-title").textContent = sel[0].name;
    $("sel-sub").textContent = sel[0].core ? "Office document" : "File dates only";
  } else if (sel.length > 1) {
    iconBox.append(el("span", "stack", String(sel.length)));
    $("sel-title").textContent = `${sel.length} files selected`;
    const parts = [];
    if (docs) parts.push(plural(docs, "Office document"));
    if (others) parts.push(plural(others, "other file"));
    $("sel-sub").textContent = parts.join(", ");
  } else {
    $("sel-title").textContent = "Nothing selected";
    $("sel-sub").textContent = state.files.length ? "Select files in the list to edit them." : "Add some files to get started.";
  }

  $("group-fs").classList.toggle("disabled", sel.length === 0);
  $("group-core").classList.toggle("disabled", docs === 0);
  $("fs-from-core").disabled = docs === 0;
  $("fs-from-core").closest(".switch-row").classList.toggle("disabled", docs === 0);
  const note = $("core-note");
  note.classList.toggle("hidden", sel.length === 0 || others === 0);
  note.textContent =
    docs === 0
      ? "None of the selected files is an Office document."
      : `Changes here apply to the ${plural(docs, "Office document")} in the selection.`;

  for (const f of fields) {
    const base = state.baselines.get(f.id) ?? { value: "", mixed: false };
    const pending = state.pending.has(f.id);
    const value = pending ? state.pending.get(f.id) : base.value;
    if (f.input.value !== value) f.input.value = value;
    renderFieldState(f, base);
  }

  renderApplyBar();
}

function renderFieldState(f, base = state.baselines.get(f.id)) {
  const pending = state.pending.has(f.id);
  f.node.classList.toggle("changed", pending && !lockedByMatch(f));
  if (f.date) {
    f.node.classList.toggle("overlaid", !f.input.value);
    f.node.querySelector(".mixed-label").textContent = base.mixed && !pending ? "Mixed values" : "Not set";
  }
  f.node.classList.toggle("locked", lockedByMatch(f));
  f.node.classList.toggle("unsupported", unsupported(f));
  if (!f.date) f.input.placeholder = base.mixed ? "Mixed values" : "Not set";
  f.input.disabled = lockedByMatch(f) || unsupported(f);
  if (unsupported(f)) f.node.title = "This system can't change file creation dates.";
}

function renderApplyBar() {
  const sel = selectedFiles();
  const changes = effectiveChanges().length + (matching() ? 1 : 0);
  const summary = $("summary");
  summary.replaceChildren();
  if (changes === 0) {
    summary.textContent = sel.length ? "No changes" : "";
  } else {
    summary.append(el("strong", null, plural(changes, "change")), ` · ${plural(sel.length, "file")}`);
  }
  $("apply").disabled = state.busy || sel.length === 0 || changes === 0;
  $("discard").disabled = state.busy || (state.pending.size === 0 && !$("fs-from-core").checked);
}

function onFieldInput(f) {
  const base = state.baselines.get(f.id);
  const v = f.input.value;
  // An emptied date means "leave as is"; a text field set to empty clears it.
  if (v === base.value || (f.date && v === "")) state.pending.delete(f.id);
  else state.pending.set(f.id, v);
  renderFieldState(f);
  renderApplyBar();
}

for (const f of fields) {
  f.input.addEventListener("input", () => onFieldInput(f));
  f.node.querySelector(".now")?.addEventListener("click", () => {
    f.input.value = toInputValue(new Date().toISOString(), state.zone);
    onFieldInput(f);
  });
  f.node.querySelector(".reset").addEventListener("click", () => {
    state.pending.delete(f.id);
    f.input.value = state.baselines.get(f.id).value;
    renderFieldState(f);
    renderApplyBar();
    f.input.focus();
  });
}

$("fs-from-core").addEventListener("change", renderInspector);

$("discard").addEventListener("click", () => {
  state.pending.clear();
  $("fs-from-core").checked = false;
  renderInspector();
});

// ---------- time zone ----------

function renderZone() {
  for (const b of document.querySelectorAll(".segmented button")) {
    b.setAttribute("aria-checked", String(b.dataset.zone === state.zone));
  }
  const local = Intl.DateTimeFormat().resolvedOptions().timeZone;
  $("zone-info").textContent =
    state.zone === "utc" ? "Dates shown and typed in UTC" : `Dates shown and typed in local time${local ? ` (${local})` : ""}`;
}

function setZone(zone) {
  if (zone === state.zone) return;
  // Re-express typed dates so they keep meaning the same instant.
  for (const f of fields) {
    if (f.date && state.pending.has(f.id)) {
      state.pending.set(f.id, toInputValue(fromInputValue(state.pending.get(f.id), state.zone), zone));
    }
  }
  state.zone = zone;
  try {
    localStorage.setItem("zone", zone);
  } catch {
    // Remembering the choice is only a convenience.
  }
  renderZone();
  recomputeBaselines();
  render();
}

for (const b of document.querySelectorAll(".segmented button")) {
  b.addEventListener("click", () => setZone(b.dataset.zone));
}

// ---------- apply ----------

async function apply() {
  if ($("apply").disabled) return;
  const sel = selectedFiles();
  const changes = effectiveChanges();
  const fromCore = matching();
  const request = { paths: sel.map((f) => f.path), zone: state.zone, fs: {}, core: {}, fsFromCore: fromCore };
  for (const [f, value] of changes) request[f.group][f.key] = value;

  const count = changes.length + (fromCore ? 1 : 0);
  const sure = await dialog.ask(
    `Apply ${plural(count, "change")} to ${plural(sel.length, "file")}?\n\nDocument properties are rewritten inside each file. This can't be undone.`,
    { title: "Apply changes", kind: "warning", okLabel: "Apply", cancelLabel: "Cancel" },
  );
  if (!sure) return;

  state.busy = true;
  renderApplyBar();
  try {
    const results = await invoke("apply_changes", { request });
    state.results.clear();
    const failed = [];
    const notes = [];
    for (const r of results) {
      const i = state.files.findIndex((f) => f.path === r.path);
      if (r.file && i >= 0) state.files[i] = r.file;
      const name = r.path.split(/[\\/]/).pop();
      if (r.ok) {
        state.results.set(r.path, "ok");
        if (r.message) notes.push(`${name}: ${r.message}`);
      } else {
        state.results.set(r.path, "error");
        if (i >= 0) state.files[i].error = r.message;
        failed.push(`${name}: ${r.message}`);
      }
    }
    if (failed.length === 0) {
      state.pending.clear();
      $("fs-from-core").checked = false;
    }
    if (failed.length > 0) {
      toast("error", `${failed.length} of ${plural(results.length, "file")} failed`, failed.join("\n"));
    } else if (notes.length > 0) {
      toast("warn", `Updated ${plural(results.length, "file")}, with notes`, notes.join("\n"));
    } else {
      toast("ok", `Updated ${plural(results.length, "file")}`);
    }
  } catch (e) {
    toast("error", "Nothing was changed", String(e));
  } finally {
    state.busy = false;
    recomputeBaselines();
    render();
  }
}

$("apply").addEventListener("click", apply);

// ---------- toast ----------

let toastTimer;
function toast(kind, title, detail) {
  const box = $("toast");
  const msg = el("div", "msg");
  msg.append(el("strong", null, title));
  if (detail) msg.append(el("small", null, detail.split("\n").slice(0, 4).join(" · ")));
  box.replaceChildren(icon({ ok: "check", warn: "alert", error: "alert" }[kind]), msg);
  box.className = `toast ${kind}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => box.classList.add("hidden"), kind === "ok" ? 3500 : 9000);
}
$("toast").addEventListener("click", () => $("toast").classList.add("hidden"));

// ---------- buttons, drag and drop, keyboard ----------

$("add-files").addEventListener("click", addFiles);
$("add-folder").addEventListener("click", addFolder);
document.querySelector('[data-action="add-files"]').addEventListener("click", addFiles);
document.querySelector('[data-action="add-folder"]').addEventListener("click", addFolder);
$("remove").addEventListener("click", removeSelected);
$("clear").addEventListener("click", () => {
  state.files = [];
  state.results.clear();
  setSelection([]);
});

// Drag and drop from the file manager. Dropped folders add their files.
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent((event) => {
  const { type } = event.payload;
  $("drop").classList.toggle("hidden", !(type === "enter" || type === "over"));
  if (type === "drop") addPaths(event.payload.paths, true);
});

document.addEventListener("keydown", (e) => {
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  const typing = e.target.matches?.("input:not([type=checkbox]), textarea");

  if (mod && key === "enter") {
    e.preventDefault();
    if (typing) e.target.blur();
    apply();
  } else if (mod && key === "o") {
    e.preventDefault();
    if (e.shiftKey) addFolder();
    else addFiles();
  } else if (mod && key === "f") {
    e.preventDefault();
    $("filter").focus();
    $("filter").select();
  } else if (typing) {
    if (key === "escape") {
      if (e.target.id === "filter" && e.target.value) {
        e.target.value = "";
        e.target.dispatchEvent(new Event("input"));
      } else {
        e.target.blur();
      }
    }
  } else if (mod && key === "a") {
    e.preventDefault();
    setSelection(state.visible.map((f) => f.path));
  } else if (key === "escape") {
    setSelection([]);
  } else if (key === "delete" || (key === "backspace" && IS_MAC)) {
    e.preventDefault();
    removeSelected();
  } else if (key === "arrowdown" || key === "arrowup") {
    e.preventDefault();
    moveCursor(key === "arrowdown" ? 1 : -1, e.shiftKey);
  }
});

// ---------- start ----------

for (const k of document.querySelectorAll("kbd[data-keys]")) {
  k.textContent = k.dataset.keys.replace("mod+", MOD).replace("Enter", "↵");
}
for (const b of document.querySelectorAll("[title*='Ctrl+']")) {
  b.title = b.title.replaceAll("Ctrl+", MOD).replace("Shift+", IS_MAC ? "⇧" : "Shift+");
}

invoke("capabilities").then((caps) => {
  state.creationTime = caps.creationTime;
  renderInspector();
});
renderZone();
recomputeBaselines();
render();
