// MetaStamp interface. Talks to the Rust side through three commands:
// load_files, apply_changes and capabilities.
const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;

const state = {
  files: [], // FileView objects from Rust, in list order
  selected: new Set(), // paths
  showUtc: false,
  creationTime: true,
};

const $ = (id) => document.getElementById(id);
const rows = $("rows");

// ---------- formatting ----------

function displayTime(iso) {
  if (!iso) return "—";
  const options = { dateStyle: "short", timeStyle: "medium" };
  if (state.showUtc) options.timeZone = "UTC";
  return new Date(iso).toLocaleString(undefined, options);
}

/** ISO instant -> value for <input type="datetime-local"> in the chosen zone. */
function toInputValue(iso, zone) {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n) => String(n).padStart(2, "0");
  const utc = zone === "utc";
  const parts = utc
    ? [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds()]
    : [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds()];
  const [y, mo, da, h, mi, s] = parts;
  return `${y}-${pad(mo)}-${pad(da)}T${pad(h)}:${pad(mi)}:${pad(s)}`;
}

function zone() {
  return document.querySelector('input[name="zone"]:checked').value;
}

function setStatus(text, kind = "") {
  const el = $("status");
  el.textContent = text;
  el.className = kind;
}

// ---------- file list ----------

function cell(text, cls) {
  const td = document.createElement("td");
  td.textContent = text;
  if (cls) td.className = cls;
  return td;
}

function render() {
  rows.replaceChildren();
  for (const f of state.files) {
    const tr = document.createElement("tr");
    tr.dataset.path = f.path;
    if (state.selected.has(f.path)) tr.classList.add("selected");
    if (f.error) tr.classList.add("error");

    const check = document.createElement("td");
    check.className = "check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = state.selected.has(f.path);
    check.append(box);
    tr.append(check);

    const name = document.createElement("td");
    name.className = "name";
    name.title = f.error ? `${f.path}\n${f.error}` : f.path;
    name.append(document.createTextNode(f.name));
    if (f.core) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "DOC";
      badge.title = "Office document: document properties can be edited";
      name.append(badge);
    }
    const sub = document.createElement("small");
    sub.textContent = f.error || f.path;
    name.append(sub);
    tr.append(name);

    tr.append(cell(displayTime(f.created)));
    tr.append(cell(displayTime(f.modified)));
    if (f.core) {
      tr.append(cell(displayTime(f.core.created)));
      tr.append(cell(displayTime(f.core.modified)));
      tr.append(cell(f.core.creator || "—"));
    } else {
      tr.append(cell("—", "na"), cell("—", "na"), cell("—", "na"));
    }
    rows.append(tr);
  }
  $("empty").classList.toggle("hidden", state.files.length > 0);
  updateControls();
}

function updateControls() {
  const n = state.selected.size;
  $("sel-count").textContent = n === 1 ? "1 file" : `${n} files`;
  $("remove").disabled = n === 0;
  $("clear").disabled = state.files.length === 0;
  $("fill").disabled = n === 0;
  $("select-all").checked = n > 0 && n === state.files.length;
  $("select-all").indeterminate = n > 0 && n < state.files.length;
  $("apply").disabled = n === 0 || !hasEdits();

  const selectedFiles = state.files.filter((f) => state.selected.has(f.path));
  const docs = selectedFiles.filter((f) => f.core).length;
  const others = selectedFiles.length - docs;
  const hints = [];
  if (others > 0 && coreEdited()) {
    hints.push(`${others} selected file(s) are not Office documents: only their file dates will change.`);
  }
  if (!state.creationTime) {
    hints.push("This system cannot change the file creation date.");
  }
  $("hint").textContent = hints.join(" ");
}

async function addPaths(paths, expandFolders) {
  if (!paths || paths.length === 0) return;
  setStatus("Reading files…");
  const views = await invoke("load_files", { paths, expandFolders });
  const known = new Set(state.files.map((f) => f.path));
  let added = 0;
  for (const v of views) {
    if (known.has(v.path)) continue;
    state.files.push(v);
    state.selected.add(v.path);
    known.add(v.path);
    added++;
  }
  render();
  setStatus(added === 1 ? "Added 1 file." : `Added ${added} files.`);
}

rows.addEventListener("click", (e) => {
  const tr = e.target.closest("tr");
  if (!tr) return;
  const path = tr.dataset.path;
  const additive = e.target.type === "checkbox" || e.ctrlKey || e.metaKey;
  if (!additive) state.selected.clear();
  if (state.selected.has(path) && additive) state.selected.delete(path);
  else state.selected.add(path);
  render();
});

$("select-all").addEventListener("change", (e) => {
  state.selected = e.target.checked ? new Set(state.files.map((f) => f.path)) : new Set();
  render();
});

$("add-files").addEventListener("click", async () => {
  const picked = await dialog.open({
    multiple: true,
    filters: [
      { name: "Office documents", extensions: ["docx", "docm", "dotx", "xlsx", "xlsm", "xltx", "pptx", "pptm", "potx"] },
      { name: "All files", extensions: ["*"] },
    ],
  });
  if (picked) await addPaths([].concat(picked), false);
});

$("add-folder").addEventListener("click", async () => {
  const picked = await dialog.open({ directory: true, multiple: true });
  if (picked) await addPaths([].concat(picked), true);
});

$("remove").addEventListener("click", () => {
  state.files = state.files.filter((f) => !state.selected.has(f.path));
  state.selected.clear();
  render();
});

$("clear").addEventListener("click", () => {
  state.files = [];
  state.selected.clear();
  render();
  setStatus("Ready.");
});

$("show-utc").addEventListener("change", (e) => {
  state.showUtc = e.target.checked;
  render();
});

// Drag and drop from the file manager. Folders dropped here add their files.
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent((event) => {
  const { type } = event.payload;
  $("drop").classList.toggle("hidden", !(type === "enter" || type === "over"));
  if (type === "drop") addPaths(event.payload.paths, true);
});

// ---------- editor ----------

const fields = [...document.querySelectorAll(".field")];

function fieldParts(field) {
  const [toggle, , input] = field.querySelectorAll("input, span");
  return { toggle, input, group: field.dataset.group, key: field.dataset.key };
}

function coreEdited() {
  return fields.some((f) => {
    const { toggle, group } = fieldParts(f);
    return group === "core" && toggle.checked;
  });
}

function hasEdits() {
  return fields.some((f) => fieldParts(f).toggle.checked) || $("fs-from-core").checked;
}

for (const field of fields) {
  const { toggle, input, group, key } = fieldParts(field);
  input.disabled = true;
  if (group === "fs" && key === "created") field.dataset.creation = "1";
  toggle.addEventListener("change", () => {
    input.disabled = !toggle.checked;
    if (toggle.checked) input.focus();
    updateControls();
  });
  input.addEventListener("input", () => {
    if (!toggle.checked) {
      toggle.checked = true;
      input.disabled = false;
    }
    updateControls();
  });
  field.querySelector(".now")?.addEventListener("click", () => {
    toggle.checked = true;
    input.disabled = false;
    input.value = toInputValue(new Date().toISOString(), zone());
    updateControls();
  });
}

$("fs-from-core").addEventListener("change", updateControls);

// Re-express typed dates when the zone changes, so they keep meaning the
// same instant.
let lastZone = zone();
for (const radio of document.querySelectorAll('input[name="zone"]')) {
  radio.addEventListener("change", () => {
    const next = zone();
    for (const field of fields) {
      const { input } = fieldParts(field);
      if (input.type !== "datetime-local" || !input.value) continue;
      const iso = lastZone === "utc" ? `${input.value}Z` : new Date(input.value).toISOString();
      input.value = toInputValue(iso, next);
    }
    lastZone = next;
  });
}

$("fill").addEventListener("click", () => {
  const first = state.files.find((f) => state.selected.has(f.path));
  if (!first) return;
  const z = zone();
  for (const field of fields) {
    const { input, group, key } = fieldParts(field);
    const source = group === "fs" ? first : first.core;
    if (!source) continue;
    const value = source[key];
    if (input.type === "datetime-local") input.value = toInputValue(value, z);
    else input.value = value ?? "";
  }
  setStatus(`Filled with the values of ${first.name}. Tick the fields you want to change.`);
});

function collectRequest() {
  const request = { paths: [...state.selected], zone: zone(), fs: {}, core: {}, fsFromCore: $("fs-from-core").checked };
  for (const field of fields) {
    const { toggle, input, group, key } = fieldParts(field);
    if (!toggle.checked) continue;
    if (input.type === "datetime-local" && !input.value) {
      throw new Error(`Enter a date for “${field.querySelector("span").textContent}”, or untick it.`);
    }
    request[group][key] = input.value;
  }
  return request;
}

$("apply").addEventListener("click", async () => {
  let request;
  try {
    request = collectRequest();
  } catch (e) {
    setStatus(e.message, "error");
    return;
  }
  const n = request.paths.length;
  const sure = await dialog.ask(
    `Change ${n === 1 ? "1 file" : `${n} files`}? Document properties are rewritten inside the file; this cannot be undone.`,
    { title: "MetaStamp", kind: "warning", okLabel: "Apply", cancelLabel: "Cancel" },
  );
  if (!sure) return;

  setStatus("Applying…");
  $("apply").disabled = true;
  try {
    const results = await invoke("apply_changes", { request });
    let failed = 0;
    const notes = [];
    for (const r of results) {
      const i = state.files.findIndex((f) => f.path === r.path);
      if (r.file && i >= 0) state.files[i] = r.file;
      if (!r.ok) {
        failed++;
        if (i >= 0) state.files[i].error = r.message;
      }
      if (r.message) notes.push(`${r.path.split(/[\\/]/).pop()}: ${r.message}`);
    }
    render();
    if (failed > 0) setStatus(`${failed} of ${n} failed. ${notes.join(" · ")}`, "error");
    else if (notes.length > 0) setStatus(`Done, with notes: ${notes.join(" · ")}`, "warn");
    else setStatus(n === 1 ? "Updated 1 file." : `Updated ${n} files.`, "ok");
  } catch (e) {
    setStatus(String(e), "error");
  } finally {
    updateControls();
  }
});

// ---------- start ----------

invoke("capabilities").then((caps) => {
  state.creationTime = caps.creationTime;
  if (!caps.creationTime) {
    const created = document.querySelector('[data-creation="1"]');
    created.querySelectorAll("input, button").forEach((el) => (el.disabled = true));
  }
  updateControls();
});
render();
