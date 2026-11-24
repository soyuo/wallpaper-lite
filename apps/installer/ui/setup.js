"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const body = document.getElementById("body");
const state = { info: null, installDir: "", launchAfter: true, launchAtLogin: false, deleteData: false };

document.getElementById("close").onclick = () => appWindow.close();
document.querySelector("[data-drag]").addEventListener("mousedown", (event) => {
  if (!event.target.closest("button")) appWindow.startDragging();
});

function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function progress(current) {
  return el("div", { className: "progress" },
    ...[1, 2, 3, 4].map((n) => el("i", { className: n <= current ? "on" : "" })),
    el("span", {}, `${current} / 4`));
}
function chips(...labels) { return el("div", { className: "chips" }, ...labels.map((l) => el("span", { className: "chip" }, l))); }
function actions(...buttons) { return el("div", { className: "actions" }, ...buttons); }
function button(label, onclick, primary = false, disabled = false) {
  return el("button", { className: "btn" + (primary ? " primary" : ""), type: "button", onclick, disabled }, label);
}
function checkRow(title, detail, key) {
  const box = el("input", { type: "checkbox", checked: state[key] });
  box.onchange = () => { state[key] = box.checked; };
  return el("label", { className: "glass-in" }, box, el("div", {}, el("strong", {}, title), detail ? el("small", {}, detail) : ""));
}
function logo() {
  const svg = document.querySelector(".titlebar svg").cloneNode(true);
  svg.removeAttribute("width"); svg.removeAttribute("height");
  svg.querySelector("linearGradient").id = "lg2";
  svg.querySelector("rect").setAttribute("fill", "url(#lg2)");
  return svg;
}
function show(...nodes) { body.replaceChildren(...nodes); }

function welcome() {
  show(
    progress(1),
    el("div", { className: "hero" }, logo(), el("h1", {}, state.info.existingInstall ? "Update wallpaper Lite" : "wallpaper Lite Setup"),
      el("p", {}, `Version ${state.info.version}`)),
    state.info.payloadReady ? "" : el("div", { className: "error", role: "alert" }, "This setup program was built without the app files, so it cannot install."),
    actions(button("Cancel", () => appWindow.close()), button("Next", path, true, !state.info.payloadReady)),
  );
}

function path() {
  const input = el("input", { value: state.installDir, spellcheck: false });
  input.setAttribute("aria-label", "Install location");
  input.oninput = () => { state.installDir = input.value; };
  const browse = button("Browse", async () => {
    const picked = await invoke("browse_folder");
    if (picked) { state.installDir = picked; input.value = picked; }
  });
  show(
    progress(2),
    el("h1", {}, "Install location"),
    chips("A running app is closed before upgrading"),
    el("div", { className: "label" }, "Install location"),
    el("div", { className: "field" }, input, browse),
    actions(button("Back", welcome), button("Install", beginInstall, true)),
  );
}

async function beginInstall() {
  const running = await invoke("running_apps").catch(() => []);
  if (!running.length) return install();
  show(
    progress(2),
    el("h1", {}, "The running app must be closed"),
    ...running.map((name) => el("div", { className: "glass-in" }, el("div", {}, el("span", { className: "mono" }, name)), el("span", { className: "chip" }, "Running"))),
    el("p", {}, "Closing it briefly returns your desktop wallpaper to normal. Your settings are kept."),
    actions(button("Cancel", () => appWindow.close()), button("Close and continue", install, true)),
  );
}

const FILES = ["wallpaper-ui.exe", "wallpaper-host.exe", "uninstall.exe", "Shortcuts and registration"];

async function install() {
  const status = Object.fromEntries(FILES.map((f) => [f, "waiting"]));
  const percent = el("span", { className: "percent" }, "0%");
  const fill = el("b");
  const rows = FILES.map((name) => ({ name, chip: el("span", { className: "chip" }, "Waiting") }));
  const redraw = () => {
    const done = FILES.filter((f) => status[f] === "done").length;
    percent.textContent = `${Math.round((done / FILES.length) * 100)}%`;
    fill.style.width = percent.textContent;
    rows.forEach(({ name, chip }) => { chip.textContent = { waiting: "Waiting", working: "Copying", done: "Copied" }[status[name]]; });
  };
  show(
    progress(3),
    el("div", { className: "head" }, el("h1", {}, "Installing"), percent),
    el("div", { className: "bar", "aria-hidden": "true" }, fill),
    ...rows.map(({ name, chip }) => el("div", { className: "glass-in" }, el("div", {}, el("span", { className: "mono" }, name)), chip)),
  );
  redraw();
  const unlisten = await listen("install-progress", ({ payload }) => { status[payload.step] = payload.status; redraw(); });
  try {
    await invoke("run_install", { options: { installDir: state.installDir, launchAtLogin: false, desktopShortcut: false } });
    FILES.forEach((f) => { status[f] = "done"; });
    redraw();
    finish();
  } catch (error) {
    show(
      progress(3),
      el("h1", {}, "Setup could not finish"),
      el("div", { className: "error", role: "alert" }, String(error)),
      actions(button("Close", () => appWindow.close()), button("Try again", path, true)),
    );
  } finally {
    unlisten();
  }
}

function finish() {
  show(
    progress(4),
    el("h1", {}, "Installation complete"),
    checkRow("Open wallpaper Lite", "", "launchAfter"),
    checkRow("Start with Windows", "Starts in the tray without a window when you sign in", "launchAtLogin"),
    actions(button("Finish", async () => {
      await invoke("set_launch_at_login", { installDir: state.installDir, enabled: state.launchAtLogin }).catch(() => {});
      if (state.launchAfter) await invoke("launch_app", { installDir: state.installDir }).catch(() => {});
      appWindow.close();
    }, true)),
  );
}

function confirmUninstall() {
  document.getElementById("title").textContent = "wallpaper Lite Uninstall";
  show(
    el("h1", {}, "Remove wallpaper Lite?"),
    el("p", {}, `This removes the app from ${state.installDir}, its shortcuts and launch at sign-in.`),
    checkRow("Also delete app data", "Your source images and videos are never deleted", "deleteData"),
    actions(button("Cancel", () => appWindow.close()), button("Remove", uninstall, true)),
  );
}

async function uninstall() {
  show(el("h1", {}, "Removing"), el("div", { className: "bar busy", "aria-hidden": "true" }, el("b")));
  try {
    await invoke("run_uninstall", { options: { deleteData: state.deleteData } });
    show(
      el("h1", {}, "wallpaper Lite was removed"),
      el("p", {}, "Windows shows your normal desktop background again."),
      actions(button("Close", () => appWindow.close(), true)),
    );
  } catch (error) {
    show(
      el("h1", {}, "Removal could not finish"),
      el("div", { className: "error", role: "alert" }, String(error)),
      actions(button("Close", () => appWindow.close(), true)),
    );
  }
}

(async () => {
  state.info = await invoke("setup_info");
  state.installDir = state.info.installDir;
  if (state.info.mode === "uninstall") confirmUninstall(); else welcome();
})();
