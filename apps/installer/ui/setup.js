"use strict";

const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const page = document.getElementById("page");
const stepList = document.getElementById("steps");
const state = { info: null, installDir: "", launchAtLogin: false, desktopShortcut: true, deleteData: false, launchAfter: true };

document.getElementById("minimize").onclick = () => appWindow.minimize();
document.getElementById("close").onclick = () => appWindow.close();
document.querySelectorAll("[data-drag]").forEach((node) => node.addEventListener("mousedown", (event) => {
  if (event.target === node || event.target.classList.contains("brand")) appWindow.startDragging();
}));

function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function steps(labels, current) {
  stepList.replaceChildren(...labels.map((label, index) =>
    el("li", { className: index < current ? "done" : index === current ? "on" : "" }, label)));
}

function toggleRow(title, detail, key) {
  const button = el("button", { className: "toggle", type: "button" });
  button.setAttribute("role", "switch");
  const sync = () => button.setAttribute("aria-checked", String(state[key]));
  button.setAttribute("aria-label", title);
  button.onclick = () => { state[key] = !state[key]; sync(); };
  sync();
  return el("div", { className: "row" }, el("div", {}, el("strong", {}, title), el("span", {}, detail)), button);
}

function actions(...buttons) { return el("div", { className: "actions" }, ...buttons); }
function button(label, onclick, primary = false, disabled = false) {
  return el("button", { className: "btn" + (primary ? " primary" : ""), type: "button", onclick, disabled }, label);
}

const INSTALL_STEPS = ["Welcome", "Options", "Installing", "Done"];
const UNINSTALL_STEPS = ["Confirm", "Removing", "Done"];

function welcome() {
  steps(INSTALL_STEPS, 0);
  const upgrade = state.info.existingInstall;
  page.replaceChildren(
    el("h1", {}, upgrade ? "Update wallpaper Lite" : "Welcome to wallpaper Lite"),
    el("p", {}, upgrade
      ? `An existing install was found. Setup will replace the app files with version ${state.info.version} and keep your library and settings.`
      : `Setup installs wallpaper Lite ${state.info.version} for your account only. It never changes your source images or videos.`),
    state.info.payloadReady ? "" : el("div", { className: "error", role: "alert" }, "This setup program was built without the app files, so it cannot install."),
    actions(button("Cancel", () => appWindow.close()), button("Next", options, true, !state.info.payloadReady)),
  );
}

function options() {
  steps(INSTALL_STEPS, 1);
  const input = el("input", { value: state.installDir, "aria-label": "Install folder", spellcheck: false });
  input.oninput = () => { state.installDir = input.value; };
  const browse = button("Browse", async () => {
    const picked = await invoke("browse_folder");
    if (picked) { state.installDir = picked; input.value = picked; }
  });
  page.replaceChildren(
    el("h1", {}, "Choose options"),
    el("div", { className: "path" }, input, browse),
    toggleRow("Start with Windows", "Launch wallpaper Lite when you sign in", "launchAtLogin"),
    toggleRow("Desktop shortcut", "Add a shortcut to your desktop", "desktopShortcut"),
    actions(button("Back", welcome), button("Install", install, true)),
  );
}

async function install() {
  steps(INSTALL_STEPS, 2);
  page.replaceChildren(
    el("h1", {}, "Installing..."),
    el("p", {}, "Stopping the running app, copying files and creating shortcuts."),
    el("div", { className: "bar", "aria-hidden": "true" }, el("i")),
  );
  try {
    await invoke("run_install", { options: { installDir: state.installDir, launchAtLogin: state.launchAtLogin, desktopShortcut: state.desktopShortcut } });
    installDone();
  } catch (error) {
    steps(INSTALL_STEPS, 2);
    page.replaceChildren(
      el("h1", {}, "Setup could not finish"),
      el("div", { className: "error", role: "alert" }, String(error)),
      actions(button("Close", () => appWindow.close()), button("Try again", options, true)),
    );
  }
}

function installDone() {
  steps(INSTALL_STEPS, 3);
  page.replaceChildren(
    el("h1", {}, "wallpaper Lite is installed"),
    toggleRow("Open wallpaper Lite now", "Start the app when you finish", "launchAfter"),
    actions(button("Finish", async () => {
      if (state.launchAfter) await invoke("launch_app", { installDir: state.installDir }).catch(() => {});
      appWindow.close();
    }, true)),
  );
}

function confirmUninstall() {
  steps(UNINSTALL_STEPS, 0);
  page.replaceChildren(
    el("h1", {}, "Remove wallpaper Lite"),
    el("p", {}, `This removes the app from ${state.installDir}, its shortcuts and launch at sign-in. Your source images and videos are never deleted.`),
    toggleRow("Delete my library and settings", "Removes the saved library, editor settings and display profiles", "deleteData"),
    actions(button("Cancel", () => appWindow.close()), button("Remove", uninstall, true)),
  );
}

async function uninstall() {
  steps(UNINSTALL_STEPS, 1);
  page.replaceChildren(
    el("h1", {}, "Removing..."),
    el("div", { className: "bar", "aria-hidden": "true" }, el("i")),
  );
  try {
    await invoke("run_uninstall", { options: { deleteData: state.deleteData } });
    steps(UNINSTALL_STEPS, 2);
    page.replaceChildren(
      el("h1", {}, "wallpaper Lite was removed"),
      el("p", {}, "Windows shows your normal desktop background again."),
      actions(button("Close", () => appWindow.close(), true)),
    );
  } catch (error) {
    page.replaceChildren(
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
