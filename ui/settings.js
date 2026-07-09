const TAURI = window.__TAURI__;
const inv = TAURI ? TAURI.core.invoke : async () => { throw "NO_TAURI"; };
const listen = TAURI ? TAURI.event.listen : async () => {};

let S = null;
let t = (k) => k;
let lang = "ru";
let dl = null; // { id, downloaded, total }
let HW = null; // рекомендация под железо { ramGb, vramGb, gpu, recommendedId }

async function loadHardware() {
  try {
    HW = await inv("recommend_model");
    if (S) renderModels();
  } catch (e) {}
}

const $ = (id) => document.getElementById(id);

async function save() {
  try {
    await inv("save_settings", { newSettings: S.settings });
    return true;
  } catch (e) {
    return String(e);
  }
}

function fmtGb(bytes) {
  return (bytes / 1024 / 1024 / 1024).toFixed(2) + " GB";
}

// ---------- рендер ----------

function renderTexts() {
  $("nav-chat").textContent = "💬 " + t("tabChat");
  $("nav-general").textContent = t("tabGeneral");
  $("nav-styles").textContent = t("tabStyles");
  $("nav-model").textContent = t("tabModel");
  $("version").textContent = "v" + S.version;
  $("privacy-note").textContent = t("privacyNote");
  $("l-hotkey").textContent = t("hotkeyLabel");
  $("l-hotkey-hint").textContent = t("hotkeyHint");
  $("hotkey-err").textContent = t("hotkeyErr");
  $("l-lang").textContent = t("langLabel");
  $("l-theme").textContent = t("themeLabel");
  const th = $("theme");
  th.options[0].text = t("themeSystem");
  th.options[1].text = t("themeLight");
  th.options[2].text = t("themeDark");
  $("l-autostart").textContent = t("autostartLabel");
  $("l-autocopy").textContent = t("autoCopyLabel");
  $("l-autocopy-hint").textContent = t("autoCopyHint");
  $("l-myexamples").textContent = t("myExamplesLabel");
  $("l-myexamples-hint").textContent = t("myExamplesHint");
  $("myexamples").placeholder = t("myExamplesPh");
  $("l-username").textContent = t("userNameLabel");
  $("l-username-hint").textContent = t("userNameHint");
  $("username").placeholder = t("userNamePh");
  $("l-useraliases").textContent = t("userAliasesLabel");
  $("useraliases").placeholder = t("userAliasesPh");
  $("l-howto").textContent = t("howtoTitle");
  $("howto1").textContent = t("howto1");
  $("howto2").innerHTML = t("howto2") + " — <b>" + S.settings.hotkey + "</b>";
  $("howto3").textContent = t("howto3");
  $("howto4").textContent = t("howto4");
  $("l-builtin").textContent = t("builtinTitle");
  $("l-custom").textContent = t("customTitle");
  $("l-style-add").textContent = t("styleName");
  $("new-style-name").placeholder = t("styleNamePh");
  $("new-style-prompt").placeholder = t("stylePromptPh");
  $("btn-add-style").textContent = t("styleAdd");
  $("welcome-note").textContent = t("welcome");
}

function renderGeneral() {
  $("hotkey").value = S.settings.hotkey;
  $("lang").value = S.settings.uiLang;
  $("theme").value = S.settings.theme;
  $("autostart").checked = S.settings.autostart;
  $("autocopy").checked = S.settings.autoCopy !== false;
  $("myexamples").value = S.settings.myExamples || "";
  $("username").value = S.settings.userName || "";
  $("useraliases").value = (S.settings.userAliases || []).join(", ");
}

function renderStyles() {
  // Строка стиля: ползунок вкл/выкл + название + промпт (+ удалить у своих)
  function styleRow(name, prompt, id, deletable) {
    const hidden = S.settings.hiddenStyles || [];
    const isOn = !hidden.includes(id);
    const d = document.createElement("div");
    d.className = "style-item";
    if (!isOn) d.style.opacity = "0.55";
    d.innerHTML =
      '<div class="head-row">' +
      '<span class="switch"><input type="checkbox" class="sw" /><span class="track"></span></span>' +
      '<span class="name"></span>' +
      (deletable ? '<button class="btn danger del" style="margin-left:auto;font-size:12px"></button>' : "") +
      "</div><div class=\"prompt\"></div>";
    d.querySelector(".name").textContent = name;
    d.querySelector(".prompt").textContent = prompt;
    const sw = d.querySelector(".sw");
    sw.checked = isOn;
    sw.onchange = async () => {
      let h = S.settings.hiddenStyles || [];
      if (sw.checked) h = h.filter((x) => x !== id);
      else h = h.concat([id]);
      S.settings.hiddenStyles = h;
      await save();
      renderStyles();
    };
    if (deletable) {
      const del = d.querySelector(".del");
      del.textContent = t("styleDelete");
      del.onclick = async () => {
        S.settings.customStyles = S.settings.customStyles.filter((x) => x.id !== id);
        if (S.settings.activeStyle === id) S.settings.activeStyle = "friendly";
        await save();
        renderStyles();
      };
    }
    return d;
  }

  const bl = $("builtin-list");
  bl.innerHTML = "";
  for (const st of S.builtinStyles) {
    const name = (STYLE_NAMES[st.id] && STYLE_NAMES[st.id][lang]) || st.id;
    bl.appendChild(styleRow(name, st.prompt, st.id, false));
  }
  const cl = $("custom-list");
  cl.innerHTML = "";
  if (!S.settings.customStyles.length) {
    const p = document.createElement("div");
    p.className = "muted";
    p.textContent = t("noCustom");
    cl.appendChild(p);
  }
  for (const st of S.settings.customStyles) {
    cl.appendChild(styleRow(st.name, st.prompt, st.id, true));
  }
}

function engineText() {
  const st = S.llama;
  if (st === "ready") return t("engineReady");
  if (st === "loading") return t("engineLoading");
  if (st === "stopped") return t("engineStopped");
  if (st.startsWith("error:")) return t("engineError") + st.slice(6);
  return st;
}

function renderModels() {
  $("engine-status").textContent = engineText();
  $("welcome-note").style.display =
    S.firstRun && !S.settings.modelFile ? "block" : "none";

  const list = $("model-list");
  list.innerHTML = "";
  for (const m of S.modelCatalog) {
    const meta = MODEL_META[m.id] || { name: { ru: m.id }, desc: { ru: "" } };
    const downloaded = S.downloaded.includes(m.file);
    const active = S.settings.modelFile === m.file;
    const isDl = dl && dl.id === m.id;

    const card = document.createElement("div");
    card.className = "model-card";
    card.innerHTML =
      '<div class="head-row"><span class="name"></span><span class="badge"></span></div>' +
      '<div class="desc"></div>' +
      '<div class="actions">' +
      '<label class="use-radio" style="display:flex;align-items:center;gap:6px;cursor:pointer">' +
      '<input type="radio" name="active-model" style="width:16px;height:16px;accent-color:var(--accent)" />' +
      '<span class="use-label"></span></label>' +
      '<button class="btn primary act-dl"></button>' +
      '<button class="btn danger act-del"></button>' +
      '<button class="btn act-cancel"></button>' +
      '<div class="progress"><div></div></div>' +
      '<span class="muted pct"></span>' +
      "</div>";

    card.querySelector(".name").textContent = meta.name[lang] || meta.name.ru;
    const badge = card.querySelector(".badge");
    if (active) { badge.textContent = t("activeBadge"); badge.className = "badge active"; }
    else if (downloaded) badge.textContent = t("downloadedBadge");
    else badge.textContent = t("sizePrefix") + (m.sizeMb / 1024).toFixed(1) + " GB";
    // Бейдж рекомендации под железо
    if (HW && HW.recommendedId === m.id && !active) {
      const rb = document.createElement("span");
      rb.className = "badge";
      rb.style.background = "var(--accent)";
      rb.style.color = "#fff";
      rb.textContent = "⭐ " + t("recommendBadge");
      card.querySelector(".head-row").appendChild(rb);
    }
    card.querySelector(".desc").textContent = meta.desc[lang] || meta.desc.ru;

    const bDl = card.querySelector(".act-dl");
    const radioWrap = card.querySelector(".use-radio");
    const radio = radioWrap.querySelector("input");
    const bDel = card.querySelector(".act-del");
    const bCancel = card.querySelector(".act-cancel");
    const prog = card.querySelector(".progress");
    const pct = card.querySelector(".pct");

    bDl.textContent = t("dlBtn");
    radioWrap.querySelector(".use-label").textContent = t("activeRadio");
    bDel.textContent = t("delBtn");
    bCancel.textContent = t("cancelBtn");

    bDl.style.display = !downloaded && !isDl ? "" : "none";
    radioWrap.style.display = downloaded ? "" : "none";
    radio.checked = active;
    bDel.style.display = downloaded ? "" : "none";
    bCancel.style.display = isDl ? "" : "none";
    prog.style.display = isDl ? "block" : "none";

    if (isDl && dl.total) {
      const p = Math.round((dl.downloaded / dl.total) * 100);
      prog.firstElementChild.style.width = p + "%";
      pct.textContent = p + "% · " + fmtGb(dl.downloaded) + " / " + fmtGb(dl.total);
    }

    bDl.onclick = async () => {
      dl = { id: m.id, downloaded: 0, total: m.sizeMb * 1024 * 1024 };
      renderModels();
      try {
        await inv("download_model", { id: m.id });
      } catch (e) {
        if (!String(e).includes("CANCELED")) alert(t("dlError") + e);
      }
      dl = null;
      await refresh();
    };
    bCancel.onclick = () => inv("cancel_download");
    radio.onchange = async () => {
      if (!radio.checked) return;
      try { await inv("use_model", { file: m.file }); } catch (e) { alert(e); }
      await refresh();
    };
    bDel.onclick = async () => {
      if (!confirm(t("confirmDelModel"))) return;
      try { await inv("delete_model", { file: m.file }); } catch (e) { alert(t("delErr") + e); }
      await refresh();
    };

    list.appendChild(card);
  }

  // Инфо про железо и рекомендацию
  const hn = $("hw-note");
  if (HW) {
    const u = lang === "ru" ? " ГБ" : " GB";
    const gpuPart =
      HW.vramGb >= 0.5 && HW.gpu
        ? t("yourPcGpu") + " " + HW.gpu + " " + HW.vramGb + u
        : t("yourPcNoGpu");
    const recMeta = MODEL_META[HW.recommendedId];
    const recName = recMeta ? recMeta.name[lang] || recMeta.name.ru : HW.recommendedId;
    hn.style.display = "";
    hn.innerHTML =
      "<b>" + t("yourPc") + "</b> " + HW.ramGb + u + " " + (lang === "ru" ? "ОЗУ" : "RAM") +
      ", " + gpuPart + ".<br>" + t("recommendLine") + " <b>" + recName + "</b>";
  } else {
    hn.style.display = "none";
  }

  // Инфо: где лежат модели и сколько занимают
  $("models-dir").textContent = S.modelsDir;
  $("models-size").textContent =
    t("totalSizeLabel") + " " + fmtGb(S.modelsBytes || 0);
  $("l-models-dir").textContent = t("modelsDirLabel");
  $("btn-open-models").textContent = t("openFolder");
}

function renderAll() {
  renderTexts();
  renderGeneral();
  renderStyles();
  renderModels();
}

async function refresh() {
  S = await inv("get_state");
  lang = S.settings.uiLang;
  t = makeT(lang);
  window.__currentTheme = S.settings.theme;
  applyTheme(S.settings.theme);
  renderAll();
  if (window.Chat) Chat.onData();
}

// ---------- вкладки ----------

function switchTab(name) {
  for (const n of ["chat", "general", "styles", "model"]) {
    $("nav-" + n).classList.toggle("active", n === name);
    $("sec-" + n).classList.toggle("active", n === name);
  }
  if (name === "chat" && window.Chat) Chat.activate();
}

// ---------- запись горячей клавиши ----------

function keyFromCode(code) {
  let m;
  if ((m = code.match(/^Key([A-Z])$/))) return m[1];
  if ((m = code.match(/^Digit(\d)$/))) return m[1];
  if ((m = code.match(/^F(\d{1,2})$/))) return "F" + m[1];
  if (code === "Space") return "Space";
  return null;
}

function setupHotkeyRecorder() {
  const inp = $("hotkey");
  const err = $("hotkey-err");
  inp.addEventListener("focus", () => {
    inp.classList.add("rec");
    inp.value = t("hotkeyPress");
  });
  inp.addEventListener("blur", () => {
    inp.classList.remove("rec");
    inp.value = S.settings.hotkey;
  });
  inp.addEventListener("keydown", async (e) => {
    e.preventDefault();
    const key = keyFromCode(e.code);
    if (!key) return;
    const mods = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    if (!mods.length) return; // нужна хотя бы одна модификаторная клавиша
    const combo = mods.concat([key]).join("+");
    const prev = S.settings.hotkey;
    S.settings.hotkey = combo;
    const res = await save();
    if (res !== true) {
      S.settings.hotkey = prev;
      err.style.display = "block";
      setTimeout(() => (err.style.display = "none"), 3000);
    } else {
      err.style.display = "none";
    }
    inp.blur();
    renderGeneral();
    renderTexts();
  });
}

// ---------- инициализация ----------

async function boot() {
  applyTheme("system");
  if (!TAURI) {
    document.body.innerHTML =
      '<div style="margin:auto;padding:40px;text-align:center">Это окно работает только внутри приложения — запусти Otvetka.exe</div>';
    return;
  }
  // Бэкенд может быть ещё не готов первые мгновения — пробуем с повторами
  let ok = false;
  for (let i = 0; i < 20; i++) {
    try {
      await refresh();
      ok = true;
      break;
    } catch (e) {
      await new Promise((r) => setTimeout(r, 300));
    }
  }
  if (!ok) return;

  loadHardware(); // определяем железо в фоне, обновит карточки моделей
  $("btn-open-models").onclick = () => inv("open_models_dir");
  if (window.Chat) await Chat.init();
  $("nav-chat").onclick = () => switchTab("chat");
  $("nav-general").onclick = () => switchTab("general");
  $("nav-styles").onclick = () => switchTab("styles");
  $("nav-model").onclick = () => switchTab("model");

  if (S.firstRun || !S.settings.modelFile) switchTab("model");

  $("lang").onchange = async (e) => {
    S.settings.uiLang = e.target.value;
    await save();
    await refresh();
  };
  $("theme").onchange = async (e) => {
    S.settings.theme = e.target.value;
    await save();
    window.__currentTheme = S.settings.theme;
    applyTheme(S.settings.theme);
  };
  $("autostart").onchange = async (e) => {
    S.settings.autostart = e.target.checked;
    await save();
  };
  $("autocopy").onchange = async (e) => {
    S.settings.autoCopy = e.target.checked;
    await save();
  };
  $("myexamples").onchange = async (e) => {
    S.settings.myExamples = e.target.value;
    await save();
  };
  $("username").onchange = async (e) => {
    S.settings.userName = e.target.value.trim();
    await save();
  };
  $("useraliases").onchange = async (e) => {
    S.settings.userAliases = e.target.value
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    await save();
  };
  $("btn-add-style").onclick = async () => {
    const name = $("new-style-name").value.trim();
    const prompt = $("new-style-prompt").value.trim();
    if (!name || !prompt) return;
    S.settings.customStyles.push({ id: "c" + Date.now(), name, prompt });
    $("new-style-name").value = "";
    $("new-style-prompt").value = "";
    await save();
    renderStyles();
  };

  setupHotkeyRecorder();

  await listen("model-dl-progress", (e) => {
    const p = e.payload;
    if (p.canceled) { dl = null; refresh(); return; }
    dl = p.done ? null : { id: p.id, downloaded: p.downloaded, total: p.total };
    renderModels();
  });
  await listen("llama-status", (e) => {
    S.llama = e.payload.status;
    renderModels();
  });
  await listen("settings-changed", () => refresh());
}

boot();
