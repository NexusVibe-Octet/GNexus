// G Nexus front-end. Talks to the Rust side through Tauri commands and the
// "g13" event stream; the edited model is the profile file itself (FileDto).
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
/** Same page, two windows: the main one and the in-game overlay (#overlay). */
const OVERLAY = location.hash === "#overlay";
if (OVERLAY) document.documentElement.classList.add("overlay");

const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const S = {
  profiles: [],
  editId: null,
  dto: null,
  bank: "M1",
  activeId: "",
  connected: false,
  offlineEdit: false,
  status: {},
  selected: null,
  mods: new Set(),
  pickerTab: "keyboard",
  advTab: "apps",
  base: {}, // bank → base colour (before brightness)
  game: null, // overlay: executable in the foreground when it was opened
  pinned: null, // profile pinned everywhere ("Tester partout")
};

// ---- helpers --------------------------------------------------------------------

let toastTimer;
function toast(msg, err = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.toggle("err", err);
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), err ? 6000 : 1800);
}

const hex2rgb = (h) => {
  const m = /^#?([0-9a-f]{6})$/i.exec(h || "");
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return { r: n >> 16, g: (n >> 8) & 255, b: n & 255 };
};
const rgb2hex = ({ r, g, b }) => "#" + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("").toUpperCase();

function bankObj(bank = S.bank) {
  S.dto.banks ||= {};
  S.dto.banks[bank] ||= {};
  return S.dto.banks[bank];
}

function bankActions(bank = S.bank) {
  const b = bankObj(bank);
  const out = {};
  for (const [k, v] of Object.entries(b)) if (k !== "color") out[k] = v;
  return out;
}

// ---- saving -----------------------------------------------------------------------

let saveTimer;
/** Colour dialog while open: { target, h, s, v } (see openColor). */
let CP = null;
function save(msg) {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    const dto = structuredClone(S.dto);
    // half-typed rules are kept in the UI but not written
    dto.rules = (dto.rules || []).filter((r) => (r.exe && r.exe.trim()) || (r.title && r.title.trim()))
      .map((r) => {
        const o = {};
        if (r.exe && r.exe.trim()) o.exe = r.exe.trim();
        if (r.title && r.title.trim()) o.title = r.title.trim();
        return o;
      });
    try {
      await invoke("save_profile", { id: S.editId, profile: dto });
      if (msg) toast(msg);
      refreshProfiles();
    } catch (e) {
      toast(t("Non enregistré : {e}", { e }), true);
    }
  }, 220);
}

// ---- profiles ---------------------------------------------------------------------

async function refreshProfiles() {
  try {
    S.profiles = await invoke("list_profiles");
  } catch (e) {
    toast(String(e), true);
    S.profiles = [];
  }
  renderProfileMenu();
  renderStatus();
}

async function loadEdit(id) {
  try {
    const dto = await invoke("load_profile", { id });
    S.editId = id;
    S.dto = dto;
    S.dto.rules ||= [];
    S.dto.macros ||= {};
    S.base = {};
    for (const b of ["M1", "M2", "M3"]) {
      const c = hex2rgb(bankObj(b).color);
      if (c) {
        const m = Math.max(c.r, c.g, c.b, 1);
        S.base[b] = { rgb: rgb2hex({ r: (c.r * 255) / m, g: (c.g * 255) / m, b: (c.b * 255) / m }), bright: Math.round((m / 255) * 100) };
      }
    }
    G13Macros.sel = null;
    // while this window has the focus, the G13 uses the profile shown here
    // (the overlay's preview is set by the back-end: the game's profile)
    if (!OVERLAY) invoke("set_preview", { id }).catch(() => {});
    if (!document.body.classList.contains("mouse-mode") && !document.body.classList.contains("headset-mode")) $("#ov-name").textContent = dto.name || id;
    render();
  } catch (e) {
    toast(t("Profil illisible : {e}", { e }), true);
  }
}

function renderProfileMenu() {
  const idx = S.profiles.findIndex((p) => p.id === S.editId);
  const cur = S.profiles[idx];
  $("#profile-num").textContent = cur ? t("PROFIL {n}", { n: idx + 1 }) : t("PROFIL");
  $("#profile-name").innerHTML = cur ? (cur.default ? '<span class="star">★</span> ' : "") + esc(cur.name) : "—";
  $("#profile-label-text").textContent = cur ? cur.name.toUpperCase() : "";
  $("#profile-label-text").title = cur ? t("Profil {n} · {name}", { n: idx + 1, name: cur.name }) : "";
  $("#profile-list").innerHTML = S.profiles
    .map((p, i) => {
      const tip = `${i + 1}. ${p.name}${p.default ? " " + t("(par défaut)") : ""}${p.apps.length ? " — " + p.apps.join(", ") : ""}${p.error ? " — " + t("fichier invalide : {e}", { e: p.error }) : ""}`;
      return `<button class="bank prof ${p.id === S.editId ? "on" : ""} ${p.error ? "err" : ""}" data-id="${esc(p.id)}" title="${esc(tip)}">
        ${i + 1}${p.id === S.activeId ? `<i class="act" title="${t("actif sur le G13")}"></i>` : ""}</button>`;
    })
    .join("") + `<button class="bank add" id="profile-add" title="${t("Nouveau profil")}">+</button>`;
  $("#btn-default").classList.toggle("on", !!cur?.default);
  renderScope(cur);
  if (OVERLAY) renderOverlayBar(cur);
}

/** Overlay head for the device shown: the G13's (renderOverlayBar) or the mouse's. */
function renderOverlayHead() {
  if (document.body.classList.contains("headset-mode")) {
    const st = Headset.status;
    $("#ov-name").textContent = st?.present ? st.name : t("Casque non détecté");
    $("#ov-game-text").textContent = st?.connected && st.battery != null ? t("Batterie : {n} %", { n: st.battery }) : "";
    $("#ov-assign").classList.add("hidden");
    return;
  }
  if (!document.body.classList.contains("mouse-mode")) {
    const cur = S.profiles.find((p) => p.id === S.editId);
    $("#ov-name").textContent = S.dto?.name || "—";
    renderOverlayBar(cur);
    return;
  }
  const m = Mouse;
  const host = m.host();
  const game = S.game;
  $("#ov-name").textContent = host ? m.softName(m.softIndex()) : m.status?.connected ? t("Profil intégré {n}", { n: m.sector() }) : t("Souris non détectée");
  const bound = host && game && (m.softProfile()?.apps || []).some((a) => a.toLowerCase() === game.toLowerCase());
  $("#ov-game-text").innerHTML = !game || !host
    ? t("Profil affiché : <b>{name}</b>", { name: esc($("#ov-name").textContent) })
    : bound
      ? t("Jeu : <b>{game}</b> · utilise ce profil", { game: esc(game) })
      : t("Jeu : <b>{game}</b> · ce profil ne lui est pas associé", { game: esc(game) });
  $("#ov-assign").classList.toggle("hidden", !game || !host || bound);
}

/** Overlay: profile shortcuts (top right) and the game line. */
function renderOverlayBar(cur) {
  if (document.body.classList.contains("mouse-mode") || document.body.classList.contains("headset-mode")) return renderOverlayHead();
  $("#ov-profiles").innerHTML = S.profiles.map((p, i) =>
    `<button class="bank prof ${p.id === S.editId ? "on" : ""}" data-id="${esc(p.id)}" title="${esc(`${i + 1}. ${p.name}`)}">
      ${i + 1}${p.id === S.activeId ? '<i class="act"></i>' : ""}</button>`).join("");
  const game = S.game;
  const bound = game && (S.dto?.rules || []).some((r) => (r.exe || "").toLowerCase() === game.toLowerCase());
  $("#ov-game-text").innerHTML = !game
    ? t("Profil affiché : <b>{name}</b>", { name: esc(cur?.name || "—") })
    : bound
      ? t("Jeu : <b>{game}</b> · utilise ce profil", { game: esc(game) })
      : t("Jeu : <b>{game}</b> · ce profil ne lui est pas associé", { game: esc(game) });
  $("#ov-assign").classList.toggle("hidden", !game || bound);
}

$("#ov-profiles").addEventListener("click", async (e) => {
  const b = e.target.closest("button[data-id]");
  if (!b || b.dataset.id === S.editId) return;
  await loadEdit(b.dataset.id);
  // while the overlay has the focus, the G13 follows the profile shown here
  invoke("set_preview", { id: b.dataset.id }).catch(() => {});
});

$("#ov-assign").addEventListener("click", async () => {
  if (S.game && document.body.classList.contains("mouse-mode")) {
    Mouse.bindApp(S.game);
    toast(t("« {name} » s'activera désormais dans {app}", { name: Mouse.softName(Mouse.softIndex()), app: S.game }));
    return;
  }
  if (!S.game || !S.editId) return;
  try {
    await invoke("assign_game", { id: S.editId, exe: S.game });
    await refreshProfiles();
    await loadEdit(S.editId);
    toast(t("« {name} » s'activera désormais dans {app}", { name: S.dto.name, app: S.game }));
  } catch (e) { toast(String(e), true); }
});

/** Tell where the edited profile applies once this window loses the focus. */
function renderScope(cur) {
  const bar = $("#scope-bar");
  if (!cur || !S.dto) { bar.classList.add("hidden"); return; }
  bar.classList.remove("hidden");
  const apps = (S.dto.rules || []).map((r) => r.exe || r.title).filter(Boolean);
  const def = S.profiles.find((p) => p.default);
  const pinned = S.profiles.find((p) => p.id === S.pinned);
  $("#scope-pin").textContent = S.pinned === cur.id ? t("ARRÊTER LE TEST") : t("TESTER PARTOUT");
  $("#scope-pin").classList.toggle("primary", S.pinned === cur.id);
  let html, warn = false;
  if (pinned) {
    html = t("Test en cours : « <b>{name}</b> » est actif sur le G13 <b>dans toutes les applications</b> (associations et profil par défaut ignorés) jusqu'à « Arrêter le test ».", { name: esc(pinned.name) });
    warn = S.pinned !== cur.id;
  } else if (apps.length) {
    html = t("« <b>{name}</b> » s'active tout seul dans : <b>{apps}</b>.", { name: esc(cur.name), apps: esc(apps.join(", ")) }) +
      (cur.default ? " " + t("Partout ailleurs aussi (profil par défaut ★).") : "");
  } else if (cur.default) {
    html = t("« <b>{name}</b> » est le profil par défaut ★ : il s'applique partout où aucun profil de jeu n'est associé.", { name: esc(cur.name) });
  } else {
    warn = true;
    html = t("Vous modifiez « <b>{name}</b> » : il est actif ici pour l'essai, mais hors de G Nexus c'est « <b>{def}</b> » ★ qui s'applique, car ce profil n'est associé à aucun jeu.",
      { name: esc(cur.name), def: esc(def ? def.name : "?") });
  }
  bar.classList.toggle("warn", warn);
  $("#scope-text").innerHTML = html;
  $("#scope-default").classList.toggle("hidden", !!cur.default);
  $("#scope-app").classList.toggle("hidden", apps.length > 0);
}

// ---- main render -------------------------------------------------------------------

function render() {
  if (!S.dto) return;
  const labels = {};
  for (const [k, v] of Object.entries(bankActions())) labels[k] = actionLabel(v);
  Device.setLabels(labels);
  Device.setBank(S.bank);
  for (const b of $$(".bank[data-bank]")) b.classList.toggle("on", b.dataset.bank === S.bank);
  $("#bank-label-text").textContent = t("MENU {bank}", { bank: S.bank });
  renderColor();
  renderStick();
  Device.setLcd([S.dto.name || "", t("Menu {bank}", { bank: S.bank }), S.connected ? "G Nexus" : t("hors ligne")]);
  renderProfileMenu();
  renderOffline();
}

function stickCfg() {
  return { mode: S.dto.stick?.mode || "keys", deadzone: S.dto.stick?.deadzone ?? 35, target: S.dto.stick?.target || "left" };
}

function renderStick() {
  const { mode, deadzone, target } = stickCfg();
  Device.setStickMode(mode, target);
  $("#device").classList.toggle("gamepad", mode !== "keys");
  for (const b of $$("#stick-mode .pill-btn")) b.classList.toggle("on", b.dataset.mode === mode);
  $("#stick-mode-label").textContent = mode === "keys" ? t("CLAVIER") : t("MANETTE");
  $("#deadzone").value = deadzone;
  $("#dz-val").textContent = deadzone + " %";
}

function setStick(patch, msg) {
  const st = { ...stickCfg(), ...patch };
  if (st.target === "left") delete st.target; // default, kept out of the file
  S.dto.stick = st;
  renderStick();
  save(msg);
}

function renderColor() {
  const dev = $("#device");
  const base = S.base[S.bank];
  const col = bankObj().color;
  const root = document.documentElement.style;
  dev.classList.toggle("no-color", !col);
  dev.style.setProperty("--glow", col && hex2rgb(col) ? col : "#a7aebb");
  // what the G13 actually shows: key legends and LCD take this colour…
  root.setProperty("--tint", col && hex2rgb(col) ? col : "#ffffff");
  dev.style.setProperty("--tint", col && hex2rgb(col) ? col : "#ffffff");
  // …and the whole app (tabs, selections, picker) uses its hue as accent
  if (base) root.setProperty("--accent", base.rgb);
  else root.removeProperty("--accent");
  if (base) {
    // glow shows the hue at a readable intensity; the device gets the dimmed value
    dev.style.setProperty("--glow", base.rgb);
    $("#bright").value = base.bright;
    $("#bright-val").textContent = base.bright + "%";
  } else {
    $("#bright").value = 100;
    $("#bright-val").textContent = "—";
  }
  $("#color-btn").title = col ? t("Couleur du menu {bank} : {color}", { bank: S.bank, color: col }) : t("Aucune couleur : celle du menu précédent est conservée");
  if (Device.lastFrame) Device.setLcdFrame(Device.lastFrame, lcdColor());
}

/** Colour of lit LCD pixels: the menu colour lightened like the key legends. */
function lcdColor() {
  const c = hex2rgb(bankObj().color) || { r: 255, g: 255, b: 255 };
  const mix = (v) => v * 0.62 + 255 * 0.38;
  return rgb2hex({ r: mix(c.r), g: mix(c.g), b: mix(c.b) });
}

function applyColor() {
  const base = S.base[S.bank];
  if (!base) return;
  const c = hex2rgb(base.rgb);
  const f = base.bright / 100;
  bankObj().color = rgb2hex({ r: c.r * f, g: c.g * f, b: c.b * f });
  renderColor();
  save();
}

function renderStatus() {
  const chip = $("#status-chip");
  const st = S.status;
  chip.classList.toggle("ok", S.connected);
  chip.classList.toggle("warn", !!st.error);
  let text = st.error ? t("Moteur arrêté") : S.connected ? t("G13 connecté") : t("G13 non détecté");
  if (st.dry_run) text += " · " + t("simulation");
  $("#status-text").textContent = text;
  chip.title = st.error || "";
  const act = S.profiles.find((p) => p.id === S.activeId);
  $("#active-profile").textContent = act ? act.name : S.activeId || "—";
}

function renderOffline() {
  const off = !S.connected && !S.offlineEdit;
  $("#device").classList.toggle("offline-mode", off);
  $("#offline").classList.toggle("show", off);
}

// ---- picker -------------------------------------------------------------------------

function openSheet(id) {
  closeSheets(false);
  $("#scrim").classList.add("show");
  $(id).classList.add("open");
}

function closeSheets(clearSel = true) {
  if (CP) return closeColor(false); // scrim / Échap on the colour dialog = Annuler
  $$(".sheet.open, .modal.open").forEach((s) => s.classList.remove("open"));
  $("#scrim").classList.remove("show");
  // a sheet closed while recording: the capture is dropped
  G13Macros.abort();
  Mouse.macroEd?.abort();
  if (clearSel) {
    S.selected = null;
    Device.setSelected(null);
    MouseDevice.setSelected(null);
  }
}

/** The G13 key `id` in the picker. */
function openPicker(id) {
  if (!S.dto) return;
  S.selected = id;
  Device.setSelected(id);
  const names = { G25: t("G25 · clic stick"), STICK_UP: "Stick ↑", STICK_DOWN: "Stick ↓", STICK_LEFT: "Stick ←", STICK_RIGHT: "Stick →" };
  openPickerFor({
    label: names[id] || id,
    sub: t("MENU {bank}", { bank: S.bank }),
    dance: true,
    get: () => bankObj()[id],
    set: (action) => {
      const b = bankObj();
      if (action == null) delete b[id];
      else b[id] = action;
      render();
      save(`${id} → ${action == null ? t("aucune action") : actionLabel(action)}`);
    },
  });
  Device.setSelected(id);
}

/** Key dragged onto another: its action replaces the target's and it becomes blank. */
function moveKey(from, to) {
  if (!S.dto || from === to) return;
  const b = bankObj();
  if (b[from] == null) { toast(t("{key} n'a aucune action à déplacer", { key: from }), true); return; }
  b[to] = b[from];
  delete b[from];
  render();
  save(t("{from} déplacé sur {to}", { from, to }));
}

/**
 * Open the action picker for any target: { label, sub, get(), set(action), mouse? }.
 * `mouse: true` limits it to what a mouse's memory can hold (one key + modifiers,
 * media keys, mouse buttons and the mouse's own functions; no macro, no gamepad).
 */
function openPickerFor(target) {
  S.pick = target;
  S.mods.clear();
  // a mouse has macros only in real time (target.macros)
  const hidden = target.mouse ? (target.macros ? ["pad"] : ["pad", "macros"]) : [];
  $$("#picker-tabs button").forEach((b) => b.classList.toggle("hidden", hidden.includes(b.dataset.tab)));
  if (hidden.includes(S.pickerTab)) S.pickerTab = "keyboard";
  $("#picker-key").textContent = target.label;
  $("#picker-bank").textContent = target.sub || "";
  // multi-state key (G13): the modes panel edits one mode at a time
  if (target.dance) {
    const cur = target.get();
    S.dance = isDance(cur) ? structuredClone(cur) : { tap: cur ?? null };
    S.danceSlot = "tap";
    S.danceTouched = false;
  }
  $("#picker").classList.toggle("with-dance", !!target.dance);
  $("#picker-dance").classList.toggle("hidden", !target.dance);
  renderPicker();
  openSheet("#picker");
}

function renderPicker() {
  const dance = S.pick?.dance;
  const whole = S.pick?.get();
  // with the modes panel, the highlighted action is the selected mode's
  const cur = dance ? S.dance[S.danceSlot] : whole;
  $("#picker-current").textContent = whole == null ? t("aucune") : isDance(whole) ? danceSummary(whole) : actionLabel(whole);
  if (dance) {
    $("#dance-grid").innerHTML = DANCE_SLOTS.map(([k, name, title]) => {
      const a = S.dance[k];
      return `<button class="dance-slot${k === S.danceSlot ? " on" : ""}" data-slot="${k}" title="${esc(t(title))}">
        <span class="ds-name">${name}</span>
        <span class="ds-act${a == null ? " empty" : ""}">${a == null ? "×" : esc(actionLabel(a))}</span>
        ${a != null ? `<span class="ds-clear" data-clear="${k}" title="${esc(t("Vider ce mode"))}">×</span>` : ""}</button>`;
    }).join("");
  }
  $$("#picker-tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === S.pickerTab));
  $("#mods").classList.toggle("hidden", !["keyboard", "basic"].includes(S.pickerTab));
  $$("#mods button").forEach((b) => b.classList.toggle("on", S.mods.has(b.dataset.mod)));
  const body = $("#picker-body");
  const isCur = (a) => (sameAction(cur, a) ? " cur" : "");

  if (S.pickerTab === "keyboard") {
    body.innerHTML = KEYBOARD_ROWS.map((row) => `<div class="kb-row">${row.map(([code, main, sub, u = 1]) => {
      const w = u * 50 + (u - 1) * 7;
      return `<button class="kb-key${isCur(code)}" data-code="${code}" style="width:${w}px" title="${code}">${esc(main)}${sub ? `<sup>${esc(sub)}</sup>` : ""}</button>`;
    }).join("")}</div>`).join("");
  } else if (S.pickerTab === "basic") {
    body.innerHTML = BASIC_GROUPS.map(([cap, codes]) => `<div class="group-cap">${t(cap)}</div><div class="tiles">${codes
      .map((c) => `<button class="tile${isCur(c)}" data-code="${c}" title="${c}">${esc(KEY_LABELS[c] || c)}</button>`).join("")}</div>`).join("");
  } else if (S.pickerTab === "media") {
    body.innerHTML = `<div class="tiles">${MEDIA.map(([c, title, icon]) =>
      `<button class="tile${isCur(c)}" data-code="${c}" title="${t(title)}">${icon}</button>`).join("")}</div>`;
  } else if (S.pickerTab === "mouse" && S.pick?.mouse) {
    // a mouse's memory: its buttons (no wheel steps) and its own functions
    body.innerHTML = `<div class="group-cap">${t("Boutons")}</div><div class="tiles">${MOUSE.map(([a, , title], i) => a.mouse
      ? `<button class="tile${isCur(a)}" data-mouse="${i}" title="${t(title)}">${esc(actionLabel(a))}</button>` : "").join("")}</div>
      ${MOUSE_SPECIAL_GROUPS.map(([cap, list]) => `<div class="group-cap">${t(cap)}</div><div class="tiles">${list.map(([id, , title]) =>
        `<button class="tile${isCur({ special: id })}" data-special="${id}" title="${esc(t(title))}">${esc(actionLabel({ special: id }))}</button>`).join("")}</div>`).join("")}`;
  } else if (S.pickerTab === "mouse") {
    body.innerHTML = `<div class="tiles">${MOUSE.map(([a, label, title], i) =>
      `<button class="tile${isCur(a)}" data-mouse="${i}" title="${t(title)}">${esc(actionLabel(a))}</button>`).join("")}</div>`;
  } else if (S.pickerTab === "pad") {
    const tip = (id, title) => (/^Bouton [ABXY]$/.test(title) ? t("Bouton {b}", { b: id }) : t(title));
    body.innerHTML = PAD_GROUPS.map(([cap, list]) => `<div class="group-cap">${t(cap)}</div><div class="tiles">${list
      .map(([id, label, title]) => `<button class="tile${isCur({ pad: id })}" data-pad="${id}" title="${esc(tip(id, title))}">${esc(label)}</button>`).join("")}</div>`).join("") +
      `<p class="muted" style="font-size:13px">${ON_LINUX
        ? t("Boutons d'une manette Xbox 360 virtuelle. Mettez le stick en mode manette pour en faire son joystick analogique.")
        : t("Boutons d'une manette Xbox 360 virtuelle (pilote ViGEmBus). Mettez le stick en mode manette pour en faire son joystick analogique.")}</p>`;
  } else {
    const all = S.pick?.macros ? S.pick.macros() : S.dto.macros || {};
    const names = Object.keys(all);
    const mode = (n) => t((MACRO_MODES.find(([m]) => m === (all[n].mode || "once")) || [])[1] || "");
    if (!MacroLib.items) MacroLib.ensure().then(() => S.pickerTab === "macros" && renderPicker());
    const lib = MacroLib.others(all);
    body.innerHTML = `<div class="tiles">${names.map((n) =>
      `<button class="tile wide${isCur({ macro: n })}" data-macro="${esc(n)}" title="${esc(mode(n))}">▶ ${esc(n)}</button>`).join("")}
      <button class="tile wide" id="macro-new-assign">${t("+ NOUVELLE MACRO")}</button></div>
      ${lib.length ? `<div class="group-cap">${t("BIBLIOTHÈQUE")}</div><div class="tiles">${lib.map((n) =>
        `<button class="tile wide lib" data-libmacro="${esc(n)}" title="${esc(t("Macro d'un autre profil : une copie est ajoutée à ce profil"))}">＋ ${esc(n)}</button>`).join("")}</div>` : ""}
      <p class="muted" style="font-size:13px">${t("Les macros se modifient dans « Paramètres avancés › Macros ».")}</p>`;
  }
}

function assign(action) {
  const target = S.pick;
  if (target?.dance) {
    // fills the selected mode; a plain one-shot pick (Tap only) still closes the sheet
    S.dance[S.danceSlot] = action;
    const plain = S.danceSlot === "tap" && !S.danceTouched && DANCE_SLOTS.every(([k]) => k === "tap" || S.dance[k] == null);
    target.set(danceValue(S.dance));
    if (plain) closeSheets();
    else { S.mods.clear(); renderPicker(); }
    return;
  }
  closeSheets();
  target?.set(action);
}

$("#dance-grid").addEventListener("click", (e) => {
  const clear = e.target.closest("[data-clear]");
  const slot = clear?.dataset.clear || e.target.closest("[data-slot]")?.dataset.slot;
  if (!slot || !S.pick?.dance) return;
  S.danceSlot = slot;
  S.danceTouched = true;
  S.mods.clear();
  if (clear) {
    S.dance[slot] = null;
    S.pick.set(danceValue(S.dance));
  }
  renderPicker();
});

$("#picker-body").addEventListener("click", (e) => {
  const t = e.target.closest("button");
  if (!t) return;
  if (t.dataset.code) {
    const keys = [...S.mods].filter((m) => m !== t.dataset.code);
    assign([...keys, t.dataset.code].join("+"));
  } else if (t.dataset.mouse) {
    assign(structuredClone(MOUSE[+t.dataset.mouse][0]));
  } else if (t.dataset.special) {
    assign({ special: t.dataset.special });
  } else if (t.dataset.pad) {
    assign({ pad: t.dataset.pad });
  } else if (t.dataset.macro) {
    assign({ macro: t.dataset.macro });
  } else if (t.dataset.libmacro) {
    // a macro of another profile: copied into this one, then bound
    const name = t.dataset.libmacro;
    const all = S.pick?.macros ? S.pick.macros() : S.dto.macros;
    if (all && MacroLib.items?.[name]) all[name] = structuredClone(MacroLib.items[name]);
    assign({ macro: name });
  } else if (t.id === "macro-new-assign" && S.pick?.newMacro) {
    const pick = S.pick;
    const name = pick.newMacro();
    assign({ macro: name });
    pick.openMacros?.();
  } else if (t.id === "macro-new-assign") {
    assign({ macro: newMacro() });
    openAdvanced("macros");
  }
});
$("#picker-tabs").addEventListener("click", (e) => {
  const t = e.target.closest("button");
  if (t) { S.pickerTab = t.dataset.tab; renderPicker(); }
});
$("#mods").addEventListener("click", (e) => {
  const t = e.target.closest("button[data-mod]");
  if (!t) return;
  const m = t.dataset.mod;
  S.mods.has(m) ? S.mods.delete(m) : S.mods.add(m);
  renderPicker();
});
$("#picker-none").addEventListener("click", () => assign(null));

// ---- advanced: rules + macros ----------------------------------------------------------

function openAdvanced(tab = S.advTab) {
  if (!S.dto) return;
  S.advTab = tab;
  $("#adv-profile").textContent = t("PROFIL « {name} »", { name: S.dto.name || "" });
  renderAdvanced();
  openSheet("#advanced");
}

function renderAdvanced() {
  $$("#adv-tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === S.advTab));
  $("#adv-apps").classList.toggle("hidden", S.advTab !== "apps");
  $("#adv-macros").classList.toggle("hidden", S.advTab !== "macros");
  if (S.advTab === "apps") { renderRules(); renderDetected(); }
  else renderMacros();
}

/** Detected applications as one-click chips (bind to the profile shown). */
async function renderDetected() {
  let list;
  try { list = await invoke("detected_apps"); } catch { list = { running: [], recent: [] }; }
  const lower = (s) => (s || "").toLowerCase();
  const mine = (S.dto.rules || []).map((r) => lower(r.exe));
  const owner = (exe) => S.profiles.find((p) => p.id !== S.editId && p.apps.some((a) => lower(a) === lower(exe)));
  const chip = (a) => {
    const bound = mine.includes(lower(a.exe));
    const other = owner(a.exe);
    return `<button class="app-chip ${bound ? "bound" : ""}" data-exe="${esc(a.exe)}" ${bound ? "disabled" : ""}
      title="${esc(a.title)}"><b>${esc(a.exe)}</b><span>${esc(a.title || "")}</span>
      ${bound ? `<em>${t("associée à ce profil")}</em>` : other ? `<em>${t("actuellement : {name}", { name: esc(other.name) })}</em>` : ""}</button>`;
  };
  $("#apps-running").innerHTML = list.running.map(chip).join("") ||
    `<span class="muted" style="font-size:13px">${t("Aucune application ouverte détectée.")}</span>`;
  $("#apps-recent").innerHTML = list.recent.map(chip).join("");
  $("#apps-recent-cap").classList.toggle("hidden", !list.recent.length);
}

async function bindApp(exe) {
  try {
    await invoke("assign_game", { id: S.editId, exe });
    await refreshProfiles();
    const keepTab = S.advTab;
    await loadEdit(S.editId);
    S.advTab = keepTab;
    renderAdvanced();
    toast(t("« {name} » s'activera dans {app}", { name: S.dto.name, app: exe }));
  } catch (e) { toast(String(e), true); }
}

for (const id of ["#apps-running", "#apps-recent"]) {
  $(id).addEventListener("click", (e) => {
    const b = e.target.closest(".app-chip");
    if (b && !b.disabled) bindApp(b.dataset.exe);
  });
}
$("#apps-refresh").addEventListener("click", renderDetected);
$("#rule-browse").addEventListener("click", async () => {
  try {
    const exe = await invoke("pick_exe");
    if (exe) await bindApp(exe);
  } catch (e) { toast(String(e), true); }
});

function renderRules() {
  $("#rules").innerHTML = S.dto.rules.map((r, i) => `<div class="rule" data-i="${i}">
      <input class="exe" data-f="exe" placeholder="programme.exe" value="${esc(r.exe || "")}" spellcheck="false">
      <input class="title" data-f="title" placeholder="${t("ou un texte du titre de la fenêtre (facultatif)")}" value="${esc(r.title || "")}" spellcheck="false">
      <button class="icon-btn danger" data-del title="${t("Retirer")}"><svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></svg></button>
    </div>`).join("") || `<p class="muted" style="font-size:13px">${t("Aucune application associée.")}</p>`;
}

$("#rules").addEventListener("input", (e) => {
  const row = e.target.closest(".rule");
  if (!row) return;
  S.dto.rules[+row.dataset.i][e.target.dataset.f] = e.target.value;
  save();
});
$("#rules").addEventListener("click", (e) => {
  if (!e.target.closest("[data-del]")) return;
  S.dto.rules.splice(+e.target.closest(".rule").dataset.i, 1);
  renderRules();
  save(t("Application retirée"));
});
$("#rule-add").addEventListener("click", () => {
  S.dto.rules.push({ exe: "" });
  renderRules();
  $$("#rules input.exe").pop()?.focus();
});

// G13 macros: the shared editor (macro-editor.js), on the profile being edited
const G13Macros = new MacroEditor({
  list: $("#macro-list"),
  edit: $("#macro-edit"),
  get: () => S.dto?.macros,
  set: (m) => { S.dto.macros = m; },
  changed: (msg) => save(msg),
  renamed: (old, nn) => {
    mapMacroUses((a) => (a.macro === old ? { macro: nn } : a));
    render();
  },
  deleted: (name) => {
    mapMacroUses((a) => (a.macro === name ? null : a));
    render();
  },
});
const renderMacros = () => G13Macros.render();

/** Rewrite every bound action of the three banks (modes of multi-state keys included); null unbinds. */
function mapMacroUses(f) {
  for (const b of ["M1", "M2", "M3"]) for (const [k, v] of Object.entries(bankActions(b))) {
    if (v == null) continue;
    let nv;
    if (isDance(v)) {
      const d = {};
      for (const [s] of DANCE_SLOTS) d[s] = v[s] != null ? f(v[s]) : null;
      nv = danceValue(d);
    } else nv = f(v);
    if (nv == null) delete bankObj(b)[k];
    else bankObj(b)[k] = nv;
  }
}
const newMacro = () => G13Macros.add();

$("#adv-tabs").addEventListener("click", (e) => {
  const t = e.target.closest("button");
  if (t) { S.advTab = t.dataset.tab; renderAdvanced(); }
});

// ---- settings ------------------------------------------------------------------------

/** Settings sections: GÉNÉRAL / CLAVIER G13 / SOURIS / CASQUE (several panes may share a tab). */
function showSettingsTab(tab) {
  $$("#set-tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === tab));
  $$("#settings .set-pane").forEach((p) => p.classList.toggle("hidden", p.dataset.pane !== tab));
}
$("#set-tabs").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-tab]");
  if (b) showSettingsTab(b.dataset.tab);
});
// About: version, the author's GitHub and Ko-fi pages (opened in the browser)
invoke("app_version").then((v) => { $("#about-version").textContent = t("version {v}", { v }); }).catch(() => {});
for (const [id, which] of [["#about-github", "github"], ["#about-kofi", "kofi"], ["#about-jasp", "jaspmakes"]]) {
  $(id).addEventListener("click", (e) => {
    e.preventDefault();
    invoke("open_link", { which }).catch((x) => toast(String(x), true));
  });
}
$$(".ms-export-btn").forEach((b) => b.addEventListener("click", async () => {
  if (!Mouse.draft) { toast(t("Souris non détectée"), true); return; }
  try {
    const path = await invoke("export_mouse_profile", { settings: Mouse.draft });
    if (path) toast(t("Profil de la souris exporté : {path}", { path }));
  } catch (e) { toast(t("Export impossible : {e}", { e }), true); }
}));
$$(".ms-import-btn").forEach((b) => b.addEventListener("click", async () => {
  if (!Mouse.draft) { toast(t("Souris non détectée"), true); return; }
  try {
    const s = await invoke("import_mouse_profile");
    if (!s) return;
    Mouse.draft = s;
    closeSheets();
    Mouse.show(true);
    Mouse.render();
    Mouse.preview(true);
    toast(Mouse.host() ? t("Profil importé : vérifiez puis « Enregistrer »") : t("Profil importé : vérifiez puis « Enregistrer dans la souris »"));
  } catch (e) { toast(t("Import impossible : {e}", { e }), true); }
}));

async function openSettings() {
  const cls = document.body.classList;
  showSettingsTab(cls.contains("mouse-mode") ? "mouse" : cls.contains("headset-mode") ? "headset" : "general");
  try {
    const s = await invoke("get_settings");
    $("#set-autostart").checked = s.autostart;
    $("#set-minimized").checked = s.start_minimized;
    $("#set-tray").checked = s.close_to_tray;
    $("#set-overlay").value = s.overlay_key || "";
    $("#set-overlay-pos").value = s.overlay_position || "right";
    $("#set-dir").textContent = s.profiles_dir || "—";
    $("#set-lang").value = s.language || "";
    $("#set-layout").value = s.keyboard_layout || "";
    $("#set-backup").value = s.auto_backup || "weekly";
    $("#set-backup-keep").value = s.auto_backup_keep || 10;
    showLastBackup(s.last_backup);
  } catch (e) { toast(String(e), true); }
  checkAccess();
  openSheet("#settings");
}

async function writeSettings() {
  try {
    await invoke("set_settings", {
      autostart: $("#set-autostart").checked,
      startMinimized: $("#set-minimized").checked,
      closeToTray: $("#set-tray").checked,
      overlayKey: $("#set-overlay").value || null,
      overlayPosition: $("#set-overlay-pos").value,
      autoBackup: $("#set-backup").value,
      autoBackupKeep: Math.max(1, Math.min(100, +$("#set-backup-keep").value || 10)),
    });
    toast(t("Paramètres enregistrés"));
  } catch (e) {
    toast(String(e), true);
    openSettings();
  }
}
function fillOverlayKeys() {
  $("#set-overlay").innerHTML = `<option value="">${t("Désactivé")}</option>` +
    Array.from({ length: 25 }, (_, i) => `G${i + 1}`)
      .map((k) => `<option value="${k}">${k === "G25" ? t("G25 · clic du stick") : k}</option>`).join("");
}
for (const id of ["#set-autostart", "#set-minimized", "#set-tray", "#set-overlay", "#set-overlay-pos", "#set-backup", "#set-backup-keep"])
  $(id).addEventListener("change", writeSettings);
function showLastBackup(date) {
  $("#set-backup-last").textContent = date ? t("Dernière sauvegarde : {date}", { date }) : t("Aucune sauvegarde pour l'instant");
}
$("#set-backup-now").addEventListener("click", async () => {
  try {
    const date = await invoke("backup_now");
    showLastBackup(date);
    toast(t("Configuration sauvegardée"));
  } catch (e) { toast(t("Sauvegarde impossible : {e}", { e }), true); }
});
$("#set-open-dir").addEventListener("click", () => invoke("open_profiles_dir").catch((e) => toast(String(e), true)));

// language and picker keyboard: saved, then both windows reload in the new language
async function writeLanguage() {
  try {
    await invoke("set_language", { language: $("#set-lang").value || null, keyboardLayout: $("#set-layout").value || null });
  } catch (e) { toast(String(e), true); }
}
$("#set-lang").addEventListener("change", writeLanguage);
$("#set-layout").addEventListener("change", writeLanguage);

$("#cfg-export").addEventListener("click", async () => {
  try {
    const path = await invoke("export_config");
    if (path) toast(t("Configuration exportée : {path}", { path }));
  } catch (e) { toast(t("Export impossible : {e}", { e }), true); }
});
$("#cfg-import").addEventListener("click", async () => {
  if (!(await ask(t("REMPLACER TOUS LES PROFILS ET PARAMÈTRES ?"), "", { confirmOnly: true, ok: t("IMPORTER") }))) return;
  try {
    const n = await invoke("import_config");
    if (n == null) return;
    sessionStorage.setItem("g13-toast", t("Configuration importée : {n} profil(s)", { n }));
    location.reload();
  } catch (e) { toast(t("Import impossible : {e}", { e }), true); }
});
$("#cfg-reset").addEventListener("click", async () => {
  if (!(await ask(t("TOUT REMETTRE PAR DÉFAUT ?"), "", { confirmOnly: true, ok: t("RÉINITIALISER") }))) return;
  try {
    await invoke("reset_config");
    sessionStorage.setItem("g13-toast", t("Configuration réinitialisée"));
    location.reload();
  } catch (e) { toast(t("Réinitialisation impossible : {e}", { e }), true); }
});
$("#cfg-backups").addEventListener("click", () => invoke("open_backups_dir").catch((e) => toast(String(e), true)));
$("#set-backup-open").addEventListener("click", () => invoke("open_backups_dir").catch((e) => toast(String(e), true)));

// ---- small dialogs ----------------------------------------------------------------------

function ask(title, value = "", { confirmOnly = false, ok = "OK", placeholder = "" } = {}) {
  return new Promise((resolve) => {
    const m = $("#prompt"), inp = $("#prompt-input");
    $("#prompt-title").textContent = title;
    inp.placeholder = placeholder;
    inp.classList.toggle("hidden", confirmOnly);
    inp.value = value;
    $("#prompt-ok").textContent = ok || "OK";
    closeSheets(false);
    $("#scrim").classList.add("show");
    m.classList.add("open");
    if (!confirmOnly) setTimeout(() => { inp.focus(); inp.select(); }, 30);
    const done = (v) => {
      m.classList.remove("open");
      $("#scrim").classList.remove("show");
      $("#prompt-ok").onclick = $("#prompt-cancel").onclick = inp.onkeydown = null;
      resolve(v);
    };
    $("#prompt-ok").onclick = () => done(confirmOnly ? true : inp.value.trim() || null);
    $("#prompt-cancel").onclick = () => done(null);
    inp.onkeydown = (e) => { if (e.key === "Enter") $("#prompt-ok").click(); if (e.key === "Escape") done(null); };
  });
}

// ---- header actions -----------------------------------------------------------------------

$("#profile-list").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  if (b.id === "profile-add") newProfile();
  else if (b.dataset.id !== S.editId) loadEdit(b.dataset.id);
});

async function newProfile() {
  const name = await ask(t("NOUVEAU PROFIL"), "", { placeholder: t("nom du jeu ou de l'application") });
  if (!name) return;
  try { const id = await invoke("create_profile", { name, from: null }); await refreshProfiles(); await loadEdit(id); toast(t("Profil créé")); }
  catch (e) { toast(String(e), true); }
}
$("#btn-new").addEventListener("click", newProfile);

$("#stick-mode").addEventListener("click", (e) => {
  const b = e.target.closest(".pill-btn");
  if (!b || !S.dto) return;
  const mode = b.dataset.mode;
  if (mode === stickCfg().mode) return;
  // a gamepad stick wants a small dead zone, keys a large one: swap the defaults
  const dz = stickCfg().deadzone;
  const deadzone = mode === "gamepad" && dz === 35 ? 15 : mode === "keys" && dz === 15 ? 35 : dz;
  setStick({ mode, deadzone }, mode === "gamepad" ? t("Stick en mode manette (joystick analogique)") : t("Stick en mode clavier"));
  if (mode === "gamepad") checkGamepad();
});

/** Warn when the virtual gamepad cannot be created (ViGEmBus missing). */
async function checkGamepad() {
  try { await invoke("gamepad_status"); }
  catch (e) {
    toast(ON_LINUX ? t("Mode manette indisponible : {e}", { e })
      : t("Mode manette indisponible : {e}. Installez le pilote ViGEmBus (github.com/nefarius/ViGEmBus).", { e }), true);
  }
}
$("#btn-dup").addEventListener("click", async () => {
  if (!S.dto) return;
  const name = await ask(t("DUPLIQUER LE PROFIL"), t("{name} (copie)", { name: S.dto.name }));
  if (!name) return;
  try { const id = await invoke("create_profile", { name, from: S.editId }); await refreshProfiles(); await loadEdit(id); toast(t("Profil dupliqué")); }
  catch (e) { toast(String(e), true); }
});
$("#btn-rename").addEventListener("click", async () => {
  if (!S.dto) return;
  const name = await ask(t("RENOMMER LE PROFIL"), S.dto.name);
  if (!name) return;
  S.dto.name = name;
  render();
  save(t("Profil renommé"));
});
function makeDefault() {
  if (!S.dto || S.dto.default) return;
  S.dto.default = true;
  save(t("Profil par défaut : il s'applique maintenant partout"));
}
$("#btn-default").addEventListener("click", makeDefault);
$("#scope-default").addEventListener("click", makeDefault);
$("#scope-pin").addEventListener("click", async () => {
  if (!S.editId) return;
  S.pinned = S.pinned === S.editId ? null : S.editId;
  await invoke("set_pinned", { id: S.pinned }).catch((e) => toast(String(e), true));
  renderProfileMenu();
  toast(S.pinned ? t("Profil actif partout jusqu'à l'arrêt du test") : t("Test arrêté : bascule automatique rétablie"));
});
$("#scope-app").addEventListener("click", () => openAdvanced("apps"));
// ---- clear bindings ------------------------------------------------------------------------
// Keys and stick directions only: colours, macros, rules and stick settings are kept.

$("#btn-clear").addEventListener("click", () => {
  if (!S.dto) return;
  $("#clear-text").textContent = t("Retire l'action de toutes les touches G et des directions du stick du profil « {name} ». Les couleurs, les macros, les applications associées et le mode du stick sont conservés.", { name: S.dto.name });
  $("#clear-bank").textContent = t("MENU {bank} SEULEMENT", { bank: S.bank });
  openSheet("#clear");
});

function clearBanks(banks) {
  for (const b of banks) {
    const obj = bankObj(b);
    for (const k of Object.keys(obj)) if (k !== "color") delete obj[k];
  }
  closeSheets();
  render();
  save(banks.length > 1 ? t("Touches de M1, M2 et M3 effacées") : t("Touches du menu {bank} effacées", { bank: banks[0] }));
}
$("#clear-bank").addEventListener("click", () => clearBanks([S.bank]));
$("#clear-all").addEventListener("click", () => clearBanks(["M1", "M2", "M3"]));

$("#btn-delete").addEventListener("click", async () => {
  if (!S.dto) return;
  const ok = await ask(t("SUPPRIMER « {name} » ?", { name: S.dto.name }), "", { confirmOnly: true, ok: t("SUPPRIMER") });
  if (!ok) return;
  try {
    await invoke("delete_profile", { id: S.editId });
    await refreshProfiles();
    const next = S.profiles.find((p) => p.id === S.activeId) || S.profiles[0];
    if (next) await loadEdit(next.id);
    toast(t("Profil supprimé"));
  } catch (e) { toast(String(e), true); }
});
// ---- Logitech Gaming Software import / export -------------------------------------------------

function showReport(title, text, items) {
  $("#report-title").textContent = title;
  $("#report-text").textContent = text;
  $("#report-list").innerHTML = items.map((s) => `<li>${esc(s)}</li>`).join("");
  $("#report-list").classList.toggle("hidden", !items.length);
  openSheet("#report");
}

$("#btn-import").addEventListener("click", async () => {
  try {
    const r = await invoke("import_lgs");
    if (!r) return; // cancelled
    await refreshProfiles();
    await loadEdit(r.id);
    if (r.skipped.length) {
      showReport("IMPORT LGS", t("Profil « {name} » importé. Ces éléments n'ont pas pu être repris :", { name: r.name }), r.skipped);
    } else {
      toast(t("Profil « {name} » importé", { name: r.name }));
    }
  } catch (e) { toast(t("Import impossible : {e}", { e }), true); }
});

$("#btn-export").addEventListener("click", async () => {
  if (!S.editId) return;
  try {
    const skipped = await invoke("export_lgs", { id: S.editId });
    if (skipped == null) return; // cancelled
    if (skipped.length) {
      showReport("EXPORT LGS", t("Profil exporté. Ces éléments ne passent pas, ou pas entièrement, dans Logitech Gaming Software :"), skipped);
    } else {
      toast(t("Profil exporté pour Logitech Gaming Software"));
    }
  } catch (e) { toast(t("Export impossible : {e}", { e }), true); }
});

$("#btn-settings").addEventListener("click", openSettings);
$("#btn-advanced").addEventListener("click", () => openAdvanced());
$("#offline-edit").addEventListener("click", () => { S.offlineEdit = true; renderOffline(); });
$("#scrim").addEventListener("click", () => closeSheets());
$$("[data-close]").forEach((b) => b.addEventListener("click", () => closeSheets()));
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  const open = document.querySelector(".sheet.open, .modal.open");
  if (open || !OVERLAY) closeSheets();
  else invoke("close_overlay");
});
$("#ov-close").addEventListener("click", () => { closeSheets(); invoke("close_overlay"); });

$("#bank-list").addEventListener("click", (e) => {
  const b = e.target.closest("[data-bank]");
  if (b) { S.bank = b.dataset.bank; render(); }
});

// ---- colour dialog -------------------------------------------------------------------------
// Live preview on the G13 while choosing; Annuler (or closing) restores the previous colour.

const COLOR_PRESETS = [["Rouge", "#FF0000"], ["Orange", "#FF6A00"], ["Jaune", "#FFD200"],
  ["Vert", "#00FF3C"], ["Turquoise", "#19E3CF"], ["Bleu", "#0050FF"]];

function hsv2hex(h, s, v) {
  const f = (n) => { const k = (n + h / 60) % 6; return v - v * s * Math.max(0, Math.min(k, 4 - k, 1)); };
  return rgb2hex({ r: f(5) * 255, g: f(3) * 255, b: f(1) * 255 });
}
function hex2hsv(hex) {
  const c = hex2rgb(hex) || { r: 25, g: 227, b: 207 };
  const r = c.r / 255, g = c.g / 255, b = c.b / 255;
  const max = Math.max(r, g, b), d = max - Math.min(r, g, b);
  let h = 0;
  if (d) h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return { h: (h * 60 + 360) % 360, s: max ? d / max : 0, v: max };
}

/**
 * Open the colour dialog for `target`: { initial, label, preview(hex), restore(), done(hex) }.
 * preview runs at every change, restore on Annuler / Échap / click outside, done on Appliquer.
 */
function openColor(target) {
  openSheet("#colorpick");
  CP = { target, ...hex2hsv(target.initial || "#19E3CF") };
  $("#cp-bank").textContent = target.label || "";
  $("#cp-presets").innerHTML = COLOR_PRESETS.map(([n, c]) =>
    `<button data-c="${c}" title="${t(n)}" style="--c:${c}"></button>`).join("");
  drawColor(false);
}

/** Redraw the dialog; `apply` also previews the colour on the G13 (and saves it). */
function drawColor(apply = true) {
  const hex = hsv2hex(CP.h, CP.s, CP.v).toUpperCase();
  const cv = $("#cp-sv"), ctx = cv.getContext("2d"), w = cv.width, h = cv.height;
  ctx.fillStyle = hsv2hex(CP.h, 1, 1); ctx.fillRect(0, 0, w, h);
  const white = ctx.createLinearGradient(0, 0, w, 0);
  white.addColorStop(0, "#fff"); white.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = white; ctx.fillRect(0, 0, w, h);
  const black = ctx.createLinearGradient(0, 0, 0, h);
  black.addColorStop(0, "rgba(0,0,0,0)"); black.addColorStop(1, "#000");
  ctx.fillStyle = black; ctx.fillRect(0, 0, w, h);
  ctx.beginPath(); ctx.arc(CP.s * w, (1 - CP.v) * h, 7, 0, Math.PI * 2);
  ctx.lineWidth = 2.5; ctx.strokeStyle = CP.v > 0.5 ? "#000" : "#fff"; ctx.stroke();
  $("#cp-hue").value = Math.round(CP.h);
  $("#cp-swatch").style.background = hex;
  if (document.activeElement !== $("#cp-hex")) $("#cp-hex").value = hex;
  $$("#cp-presets button").forEach((b) => b.classList.toggle("on", b.dataset.c === hex));
  if (apply) CP.target.preview(hex);
}

function closeColor(keep) {
  if (!CP) return;
  const { target } = CP;
  const hex = hsv2hex(CP.h, CP.s, CP.v).toUpperCase();
  CP = null;
  if (keep) target.done(hex);
  else target.restore();
  closeSheets();
}

/** The G13's colour for the menu shown (live on the device while choosing). */
function openBankColor() {
  if (!S.dto) return;
  const cur = S.base[S.bank];
  const prev = { base: cur ? { ...cur } : null, color: bankObj().color ?? null };
  openColor({
    initial: cur?.rgb,
    label: t("MENU {bank}", { bank: S.bank }),
    preview: (hex) => {
      S.base[S.bank] = { rgb: hex, bright: S.base[S.bank]?.bright ?? 100 };
      applyColor();
    },
    restore: () => {
      if (prev.base) S.base[S.bank] = prev.base; else delete S.base[S.bank];
      if (prev.color) bankObj().color = prev.color; else delete bankObj().color;
      renderColor();
      save();
    },
    done: () => toast(t("Couleur du menu {bank} : {color}", { bank: S.bank, color: bankObj().color })),
  });
}

$("#cp-presets").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-c]");
  if (b) { Object.assign(CP, hex2hsv(b.dataset.c)); drawColor(); }
});
$("#cp-hue").addEventListener("input", (e) => { CP.h = +e.target.value; drawColor(); });
$("#cp-hex").addEventListener("input", (e) => {
  const v = e.target.value.trim();
  if (/^#?[0-9a-f]{6}$/i.test(v)) { Object.assign(CP, hex2hsv(v.startsWith("#") ? v : "#" + v)); drawColor(); }
});
{
  const pick = (e) => {
    const r = $("#cp-sv").getBoundingClientRect();
    CP.s = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    CP.v = 1 - Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
    drawColor();
  };
  $("#cp-sv").addEventListener("pointerdown", (e) => { $("#cp-sv").setPointerCapture(e.pointerId); pick(e); });
  $("#cp-sv").addEventListener("pointermove", (e) => { if (e.buttons & 1) pick(e); });
}
$("#cp-ok").addEventListener("click", () => closeColor(true));
$("#cp-cancel").addEventListener("click", () => closeColor(false));

// ---- Logitech software running alongside ----------------------------------------------------

function renderConflicts(list) {
  const products = [...new Set(list.map((c) => c.product))];
  $("#conflict-banner").classList.toggle("hidden", !products.length);
  if (!products.length) return;
  const who = products.join(t(" et "));
  $("#conflict-text").textContent = products.length > 1
    ? t("{who} sont lancés : ils captent les touches du G13, qui ne répondront pas ici. Fermez-les, puis désactivez leur lancement automatique (Gestionnaire des tâches › Applications de démarrage) ou désinstallez-les. (Logitech G HUB, lui, peut rester ouvert.)", { who })
    : t("{who} est lancé : il capte les touches du G13, qui ne répondront pas ici. Fermez-le, puis désactivez son lancement automatique (Gestionnaire des tâches › Applications de démarrage) ou désinstallez-le. (Logitech G HUB, lui, peut rester ouvert.)", { who });
}

async function checkConflicts() {
  try { renderConflicts(await invoke("conflicting_software")); } catch { /* mock / unavailable */ }
}

$("#conflict-close").addEventListener("click", async () => {
  try {
    const left = await invoke("close_conflicting_software");
    renderConflicts(left);
    toast(left.length ? t("Certains processus n'ont pas pu être fermés (droits administrateur ?)") : t("Logiciels Logitech fermés"), !!left.length);
  } catch (e) { toast(String(e), true); }
});

// ---- Linux: access to the devices (udev rules), container ------------------------------------

let ACCESS = null;
function renderAccess(a) {
  ACCESS = a;
  const banner = $("#access-banner");
  $("#set-access").classList.toggle("hidden", !a);
  if (!a) { banner.classList.add("hidden"); return; }
  const missing = [...a.blocked];
  if (!a.uinput) missing.push(t("le clavier virtuel (/dev/uinput)"));
  let text = "";
  if (a.container) {
    text = t("G Nexus tourne dans un conteneur (distrobox, toolbox) : le démarrage à l'ouverture de session, l'heure du LCD et l'overlay peuvent mal fonctionner. Lancez l'AppImage de G Nexus directement sur le système, hors du conteneur.");
  } else if (missing.length) {
    text = t("G Nexus n'a pas accès à : {who}. Autorisez l'accès une fois (le mot de passe administrateur est demandé), puis rebranchez l'appareil si besoin.", { who: missing.join(", ") });
  }
  banner.classList.toggle("hidden", !text);
  $("#access-text").textContent = text;
  $("#access-grant").classList.toggle("hidden", a.container || !missing.length);
  // Settings › General
  $("#set-access-state").textContent = a.container ? t("dans un conteneur : non disponible")
    : missing.length ? t("manquant : {who}", { who: missing.join(", ") })
    : a.record ? t("OK (RECORD compris)") : t("OK (RECORD non autorisé)");
  $("#set-access-grant").classList.toggle("hidden", a.container || !missing.length);
  $("#set-access-record").classList.toggle("hidden", a.container || a.record);
}

async function checkAccess() {
  try { renderAccess(await invoke("device_access")); } catch { renderAccess(null); /* mock / unavailable */ }
}

async function grantAccess(record) {
  const buttons = ["#access-grant", "#set-access-grant", "#set-access-record"].map((id) => $(id));
  buttons.forEach((b) => { b.disabled = true; });
  try {
    const restart = await invoke("install_device_access", { record });
    if (restart) {
      sessionStorage.setItem("g13-toast", t("Accès aux appareils autorisé"));
      toast(t("Accès autorisé : G Nexus redémarre…"));
      setTimeout(() => invoke("restart_app").catch(() => {}), 1200);
      return;
    }
    await checkAccess();
    toast(ACCESS && (ACCESS.blocked.length || !ACCESS.uinput)
      ? t("Règles installées : rebranchez l'appareil pour terminer") : t("Accès aux appareils autorisé"));
  } catch (e) {
    toast(t("Accès non autorisé : {e}", { e }), true);
  } finally {
    buttons.forEach((b) => { b.disabled = false; });
  }
}
$("#access-grant").addEventListener("click", () => grantAccess(!!(ACCESS && ACCESS.record)));
$("#set-access-grant").addEventListener("click", () => grantAccess(!!(ACCESS && ACCESS.record)));
$("#set-access-record").addEventListener("click", () => grantAccess(true));

// ---- boot ------------------------------------------------------------------------------------

async function boot() {
  // language and keyboard before anything is drawn
  let settings = {};
  try { settings = await invoke("get_settings"); } catch { /* mock */ }
  LANG = pickLang(settings.language);
  if (!OVERLAY) invoke("ui_language", { lang: LANG }).catch(() => {});
  applyLayout(settings.keyboard_layout || LANG_LAYOUT[LANG], LANG);
  translateDom();
  fillOverlayKeys();
  listen("language-changed", () => location.reload());
  const pending = sessionStorage.getItem("g13-toast");
  if (pending) { sessionStorage.removeItem("g13-toast"); setTimeout(() => toast(pending), 300); }

  Device.build($("#device"), {
    onKey: openPicker,
    onBank: (b) => { S.bank = b; render(); },
    onMove: moveKey,
  });
  const fit = () => Device.fit($("#device-wrap"));
  new ResizeObserver(fit).observe($("#device-wrap"));
  fit();

  $("#color-btn").addEventListener("click", openBankColor);
  $("#color-wheel").addEventListener("click", openBankColor);
  Mouse.init();
  Headset.init();
  $("#bright").addEventListener("input", (e) => {
    if (!S.base[S.bank]) S.base[S.bank] = { rgb: "#FFFFFF", bright: 100 };
    S.base[S.bank].bright = +e.target.value;
    applyColor();
  });
  $("#device").addEventListener("click", (e) => {
    const b = e.target.closest(".pad-ctl button");
    if (b && S.dto) setStick({ target: b.dataset.target }, b.dataset.target === "left" ? t("Stick → joystick gauche") : t("Stick → joystick droit"));
  });
  $("#deadzone").addEventListener("input", (e) => {
    if (S.dto) setStick({ deadzone: +e.target.value });
  });

  await listen("g13", ({ payload: ev }) => {
    switch (ev.type) {
      case "key": Device.setLive(ev.key, ev.pressed); break;
      case "bank": S.bank = ev.bank; render(); break;
      case "profile": S.activeId = ev.id; refreshProfiles(); break;
      case "connected": S.connected = ev.value; if (ev.value) S.offlineEdit = false; render(); renderStatus(); break;
      case "record": Device.setRecording(ev.value); break;
      case "lcd": Device.setLcdFrame(ev.frame, S.dto ? lcdColor() : "#ffffff"); break;
      case "pinned": S.pinned = ev.id || null; renderProfileMenu(); break;
      case "gamepad_error": toast(t("Manette virtuelle : {m}", { m: ev.message }), true); break;
      case "mouse": Mouse.update(ev.status); break;
      case "headset": Headset.update(ev.status); break;
    }
  });

  invoke("key_names").then((names) => {
    const missing = ALL_CODES.filter((c) => !names.includes(c));
    if (missing.length) console.error("catalog.js: unknown key names", missing);
  }).catch(() => {});

  S.status = await invoke("get_status");
  S.connected = S.status.connected;
  S.activeId = S.status.active_profile;
  S.bank = S.status.bank || "M1";
  S.pinned = S.status.pinned || null;
  await refreshProfiles();
  if (S.status.lcd) Device.lastFrame = S.status.lcd;

  if (OVERLAY) {
    // opened by the overlay key (the window is created then): show the game's profile and menu
    const open = async (payload) => {
      closeSheets();
      S.game = payload.game || null;
      S.bank = payload.bank || "M1";
      // the mouse shows the profile it is using (the game's)
      if (!Mouse.draft || !Mouse.dirty()) {
        Mouse.editSoft = null;
        Mouse.editSector = null;
        Mouse.draft = structuredClone(Mouse.slotSettings() ?? null);
        Mouse.render();
      }
      await refreshProfiles();
      await loadEdit(payload.profile);
    };
    // the page is loaded: ask what to show (the window appears at the same time)
    const payload = await invoke("overlay_ready").catch(() => null);
    if (payload) {
      await open(payload);
      render();
      renderStatus();
      invoke("get_settings").then((s) => { $("#ov-key").textContent = s.overlay_key || "G25"; }).catch(() => {});
      return;
    }
    invoke("get_settings").then((s) => { $("#ov-key").textContent = s.overlay_key || "G25"; }).catch(() => {});
  } else {
    checkConflicts();
    setInterval(checkConflicts, 15000);
    checkAccess();
    setInterval(checkAccess, 5000);
    // the overlay may have edited a profile and changed the preview: take them back
    await listen("overlay-closed", async () => {
      await refreshProfiles();
      if (S.editId) await loadEdit(S.editId);
    });
  }

  const first = S.profiles.find((p) => p.id === S.activeId) || S.profiles.find((p) => p.default) || S.profiles[0];
  if (first) await loadEdit(first.id);
  render();
  renderStatus();
}

boot().catch((e) => toast(t("Erreur au démarrage : {e}", { e }), true));
