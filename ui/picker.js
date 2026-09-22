/* ==================================================================
   Working-dir picker — an in-webview folder browser.

   Why not a native dialog: tauri-plugin-dialog would add a dependency
   AND a dialog the Playwright harness cannot drive (the TEST-MAP keeps
   human-only smoke clicks rare by design). The grove stays thin: this
   file only walks `grove_list_dirs` listings and reports one path.

   Contract: window.grovePickDirectory(startPath) -> Promise<string|null>
   (null = cancelled). Self-owns its overlay; no markup in index.html.
   IIFE on purpose: classic scripts share one global lexical scope, and
   main.js must stay free to declare its own `invoke`.
   ================================================================== */

(() => {
  const { invoke } = window.__TAURI__.core;

  const picker = {
    overlay: null, pathEl: null, listEl: null, statusEl: null,
    useBtn: null, upBtn: null, current: null, resolve: null, open: false,
    invoker: null,
  };

  function pickerDone(path) {
    picker.open = false;
    picker.overlay?.classList.add("hidden");
    picker.resolve?.(path);
    picker.resolve = null;
    // hand focus back to whoever opened the dialog — keyboard and screen
    // reader users must not be stranded where the dialog left them
    if (picker.invoker && document.contains(picker.invoker)) picker.invoker.focus();
    picker.invoker = null;
  }

  /* Render one listing payload ({path, parent, dirs, total}). */
  function pickerShow(data) {
    picker.current = data.path;
    picker.pathEl.textContent = data.path;
    picker.pathEl.title = data.path;
    picker.upBtn.disabled = !data.parent;
    picker.listEl.textContent = "";
    if (!data.dirs.length) {
      const none = document.createElement("div");
      none.className = "pk-empty";
      none.textContent = "no subdirectories here";
      picker.listEl.appendChild(none);
    }
    for (const name of data.dirs) {
      const item = document.createElement("button");
      item.className = "pk-item";
      item.textContent = name;
      item.title = name;
      item.addEventListener("click", () => pickerEnter(name));
      picker.listEl.appendChild(item);
    }
    if (data.total > data.dirs.length) {
      const more = document.createElement("div");
      more.className = "pk-empty";
      more.textContent = "…and " + (data.total - data.dirs.length) + " more (capped)";
      picker.listEl.appendChild(more);
    }
    picker.useBtn.textContent = "use this directory";
    picker.useBtn.disabled = false;
  }

  async function pickerEnter(name) {
    const next = picker.current.replace(/\/+$/, "") + "/" + name;
    await pickerLoad(next);
  }

  async function pickerLoad(path) {
    picker.statusEl.textContent = "listing…";
    picker.useBtn.disabled = true;
    try {
      pickerShow(await invoke("grove_list_dirs", { path }));
      picker.statusEl.textContent = "";
    } catch (err) {
      picker.statusEl.textContent = String(err).split("\n")[0];
      picker.useBtn.disabled = false;
    }
    picker.listEl.scrollTop = 0;
  }

  function pickerBuild() {
    const overlay = document.createElement("div");
    overlay.id = "dir-picker";
    overlay.className = "picker-overlay hidden";
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    overlay.setAttribute("aria-label", "choose the working directory");
    overlay.innerHTML =
      '<div class="picker">' +
      '  <div class="pk-head">' +
      '    <span class="pk-title">choose the working directory</span>' +
      '    <button class="pk-close" title="cancel (esc)">&times;</button>' +
      "  </div>" +
      '  <div class="pk-pathrow">' +
      '    <button class="pk-up" title="up one level">&uarr;</button>' +
      '    <span class="pk-path"></span>' +
      "  </div>" +
      '  <div class="pk-list" tabindex="-1"></div>' +
      '  <div class="pk-foot">' +
      '    <span class="pk-status"></span>' +
      '    <button class="btn ghost pk-cancel">cancel</button>' +
      '    <button class="btn solid pk-use">use this directory</button>' +
      "  </div>" +
      "</div>";
    document.body.appendChild(overlay);

    picker.overlay = overlay;
    picker.pathEl = overlay.querySelector(".pk-path");
    picker.listEl = overlay.querySelector(".pk-list");
    picker.statusEl = overlay.querySelector(".pk-status");
    picker.useBtn = overlay.querySelector(".pk-use");
    picker.upBtn = overlay.querySelector(".pk-up");

    overlay.querySelector(".pk-close").addEventListener("click", () => pickerDone(null));
    overlay.querySelector(".pk-cancel").addEventListener("click", () => pickerDone(null));
    picker.useBtn.addEventListener("click", () => pickerDone(picker.current));
    picker.upBtn.addEventListener("click", () => {
      const parent = picker.current.replace(/\/+$/, "").split("/").slice(0, -1).join("/");
      pickerLoad(parent || "/");
    });
    // click the dim backdrop (not the panel) = cancel
    overlay.addEventListener("mousedown", (ev) => {
      if (ev.target === overlay) pickerDone(null);
    });
    // document-level Escape: the overlay only hears keys it has focus for
    document.addEventListener("keydown", (ev) => {
      if (picker.open && ev.key === "Escape") pickerDone(null);
    });
    return overlay;
  }

  window.grovePickDirectory = async function (startPath) {
    if (!picker.overlay) pickerBuild();
    picker.invoker = document.activeElement;
    picker.open = true;
    picker.overlay.classList.remove("hidden");
    return new Promise((resolve) => {
      picker.resolve = resolve;
      const seed = (startPath || "").trim();
      pickerLoad(seed || "/").then(() => {
        if (picker.statusEl.textContent) {
          // seed path unreadable (moved/renamed): fall back to the shell default
          invoke("grove_default_cwd")
            .then((fallback) => pickerLoad(fallback))
            .catch(() => pickerLoad("/"));
        }
      });
      picker.listEl.focus();
    });
  };
})();
