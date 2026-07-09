const TAURI = window.__TAURI__;
const inv = TAURI ? TAURI.core.invoke : async () => { throw "NO_TAURI"; };
const listen = TAURI ? TAURI.event.listen : async () => {};

let S = null;          // состояние приложения (get_state)
let t = (k) => k;      // функция перевода
let lang = "ru";
let lastText = "";
let reqToken = 0;
let variants = [];

// потоковая генерация
let streamRaw = "";
let latestGen = 0;
let streaming = false;
let boxEls = [];

const $ = (id) => document.getElementById(id);

function cleanVar(s) {
  return s.trim().replace(/^["«»“”'`]+/, "").replace(/["«»“”'`]+$/, "").trim();
}

// Разбор потока «1. … 2. … 3. …» на лету (зеркалит логику бэкенда)
function parseVariants(raw) {
  let content = raw;
  const ti = content.lastIndexOf("</think>");
  if (ti >= 0) content = content.slice(ti + 8);
  const vars = [];
  let cur = null;
  for (const line of content.split("\n")) {
    const tl = line.trim();
    const numbered =
      tl.length >= 2 && tl[0] >= "0" && tl[0] <= "9" && (tl[1] === "." || tl[1] === ")");
    if (numbered) {
      if (cur !== null && cur.trim()) vars.push(cleanVar(cur));
      cur = tl.slice(2).replace(/^\s+/, "");
    } else if (cur !== null && tl) {
      cur += " " + tl;
    }
  }
  if (cur !== null && cur.trim()) vars.push(cleanVar(cur));
  if (!vars.length) {
    for (const l of content.split("\n")) {
      const c = cleanVar(l);
      if (c) vars.push(c);
    }
  }
  return vars.slice(0, 3);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Бэкенд может быть ещё не готов в первые мгновения после старта — пробуем с повторами
async function ensureState(tries) {
  for (let i = 0; i < tries; i++) {
    try {
      await refresh();
      return true;
    } catch (e) {
      await sleep(300);
    }
  }
  return false;
}

async function refresh() {
  S = await inv("get_state");
  lang = S.settings.uiLang;
  t = makeT(lang);
  window.__currentTheme = S.settings.theme;
  applyTheme(S.settings.theme);
  $("title").textContent = t("popupTitle");
  $("hint").textContent = t("escHint");
  $("regen-label").textContent = t("regen");
  $("short-label").textContent = t("shortLabel");
  $("short-cb").checked = !!S.settings.replyShort;
  renderChips();
}

function styleList() {
  const hidden = S.settings.hiddenStyles || [];
  const builtins = S.builtinStyles
    .filter((s) => !hidden.includes(s.id))
    .map((s) => ({
      id: s.id,
      name: (STYLE_NAMES[s.id] && STYLE_NAMES[s.id][lang]) || s.id,
    }));
  const custom = S.settings.customStyles
    .filter((s) => !hidden.includes(s.id))
    .map((s) => ({ id: s.id, name: s.name }));
  return builtins.concat(custom);
}

function renderChips() {
  const box = $("chips");
  box.innerHTML = "";
  const list = styleList();
  // если активный стиль скрыли/удалили — переключаемся на первый видимый
  if (list.length && !list.some((s) => s.id === S.settings.activeStyle)) {
    S.settings.activeStyle = list[0].id;
  }
  for (const st of list) {
    const b = document.createElement("button");
    b.className = "chip" + (st.id === S.settings.activeStyle ? " active" : "");
    b.textContent = st.name;
    b.onclick = async () => {
      S.settings.activeStyle = st.id;
      renderChips();
      inv("save_settings", { newSettings: S.settings }).catch(() => {});
      if (lastText.trim()) generate();
    };
    box.appendChild(b);
  }
}

function showMessage(html, actions) {
  const c = $("content");
  c.innerHTML = "";
  const div = document.createElement("div");
  div.className = "msg";
  div.innerHTML = html;
  if (actions && actions.length) {
    const row = document.createElement("div");
    row.className = "actions";
    for (const a of actions) {
      const b = document.createElement("button");
      b.className = "btn" + (a.primary ? " primary" : "");
      b.textContent = a.label;
      b.onclick = a.onClick;
      row.appendChild(b);
    }
    div.appendChild(row);
  }
  c.appendChild(div);
}

function showLoading() {
  const c = $("content");
  c.innerHTML = "";
  for (let i = 0; i < 3; i++) {
    const d = document.createElement("div");
    d.className = "skel";
    c.appendChild(d);
  }
}

// Создаёт 3 пустых пузыря для потока, ссылки на них — в boxEls
function makeStreamBoxes() {
  const c = $("content");
  c.innerHTML = "";
  boxEls = [];
  for (let i = 0; i < 3; i++) {
    const b = document.createElement("button");
    b.className = "variant skel-empty";
    const num = document.createElement("span");
    num.className = "num";
    num.textContent = i + 1;
    const span = document.createElement("span");
    span.className = "vtext";
    b.appendChild(num);
    b.appendChild(span);
    c.appendChild(b);
    boxEls.push({ box: b, span });
  }
}

// Обновляет пузыри по текущему разбору потока
function renderStream(vars, done) {
  variants = vars;
  for (let i = 0; i < 3; i++) {
    const el = boxEls[i];
    if (!el) continue;
    const v = vars[i];
    if (v !== undefined) {
      el.span.textContent = v;
      el.box.classList.remove("skel-empty");
      el.box.classList.toggle("cursor", !done && i === vars.length - 1);
      el.box.onclick = () => pick(el.box, v);
    } else {
      el.span.textContent = "";
      el.box.classList.add("skel-empty");
      el.box.classList.remove("cursor");
      el.box.onclick = null;
    }
  }
}

// Финальный рендер (после завершения потока): ровно столько пузырей, сколько вариантов
function renderVariants(vars) {
  variants = vars;
  const c = $("content");
  c.innerHTML = "";
  vars.forEach((v, i) => {
    const b = document.createElement("button");
    b.className = "variant";
    const num = document.createElement("span");
    num.className = "num";
    num.textContent = i + 1;
    b.appendChild(num);
    b.appendChild(document.createTextNode(v));
    b.onclick = () => pick(b, v);
    c.appendChild(b);
  });
}

function onGenToken(p) {
  if (!streaming) return;
  if (p.gen < latestGen) return;
  latestGen = p.gen;
  streamRaw += p.text;
  renderStream(parseVariants(streamRaw), false);
}

function onGenDone(p) {
  if (p.gen < latestGen) return;
  streaming = false;
  const vars = parseVariants(streamRaw);
  if (!vars.length) {
    showMessage(t("genError"), [{ label: t("retry"), primary: true, onClick: generate }]);
    return;
  }
  renderVariants(vars);
}

async function pick(el, text) {
  try {
    await inv("copy_text", { text });
  } catch (e) {
    return;
  }
  el.classList.add("copied");
  $("hint").style.display = "none";
  const toast = $("toast");
  toast.textContent = t("copied");
  toast.style.display = "block";
  setTimeout(() => {
    inv("hide_popup");
    toast.style.display = "none";
    $("hint").style.display = "block";
  }, 700);
}

async function generate() {
  if (!S && !(await ensureState(5))) return;
  streamRaw = "";
  streaming = true;
  makeStreamBoxes();
  try {
    await inv("generate_stream", {
      text: lastText,
      styleId: S.settings.activeStyle,
    });
    // завершение и рендер — по событию gen-done
  } catch (err) {
    streaming = false;
    const m = String(err);
    if (m.includes("NO_MODEL")) {
      showMessage(t("noModel"), [
        { label: t("openSettings"), primary: true, onClick: () => inv("open_settings") },
      ]);
    } else if (m.includes("LOADING")) {
      showMessage(t("modelLoading"), [
        { label: t("retry"), primary: true, onClick: generate },
      ]);
    } else {
      showMessage(t("genError") + "<br><small>" + m.replace(/</g, "&lt;") + "</small>", [
        { label: t("retry"), primary: true, onClick: generate },
      ]);
    }
  }
}

async function onRequest(text) {
  lastText = text || "";
  if (!S) await ensureState(5);
  if (!lastText.trim()) {
    showMessage(t("emptyClip"));
  } else {
    generate();
  }
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") inv("hide_popup");
  if (["1", "2", "3"].includes(e.key)) {
    const i = Number(e.key) - 1;
    const els = document.querySelectorAll(".variant");
    if (els[i] && variants[i]) pick(els[i], variants[i]);
  }
});

async function boot() {
  applyTheme("system"); // тема сразу, до похода в бэкенд
  $("btn-close").onclick = () => inv("hide_popup");
  $("btn-settings").onclick = () => inv("open_settings");
  $("btn-regen").onclick = () => { if (lastText.trim()) generate(); };
  $("short-cb").onchange = (e) => {
    S.settings.replyShort = e.target.checked;
    inv("save_settings", { newSettings: S.settings }).catch(() => {});
    if (lastText.trim()) generate();
  };

  if (!TAURI) {
    $("content").innerHTML =
      '<div class="msg">Это окно работает только внутри приложения — запусти PodskazhiOtvet.exe</div>';
    return;
  }
  await ensureState(20);
  await listen("generate-request", (e) => onRequest(e.payload.text));
  await listen("settings-changed", () => refresh().catch(() => {}));
  await listen("gen-token", (e) => onGenToken(e.payload));
  await listen("gen-done", (e) => onGenDone(e.payload));
}

boot();
