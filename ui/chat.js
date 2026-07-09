// Логика встроенного чата с ИИ. Использует общий движок (активную модель).
// Полагается на глобальные t / lang / S из settings.js (вызывается после их инициализации).

const Chat = (function () {
  const cinv = window.__TAURI__ ? window.__TAURI__.core.invoke : async () => {};
  const clisten = window.__TAURI__ ? window.__TAURI__.event.listen : async () => {};
  const $ = (id) => document.getElementById(id);

  let convs = [];
  let activeId = null;
  let streaming = false;
  let curContent = "";
  let curThink = "";
  let botEl = null;
  let inited = false;

  function uid() {
    return "c" + Date.now() + Math.floor(Math.random() * 1000);
  }
  function active() {
    return convs.find((c) => c.id === activeId) || null;
  }
  function esc(s) {
    return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  const saveSoon = (() => {
    let tmr = null;
    return () => {
      clearTimeout(tmr);
      tmr = setTimeout(() => cinv("chat_set", { data: { conversations: convs } }).catch(() => {}), 400);
    };
  })();

  function modelName(file) {
    if (!S) return file;
    const m = (S.modelCatalog || []).find((x) => x.file === file);
    if (m && MODEL_META[m.id]) return MODEL_META[m.id].name[lang] || MODEL_META[m.id].name.ru;
    return file.replace(/\.gguf$/i, "");
  }

  function renderConvs() {
    const box = $("chat-convs");
    box.innerHTML = "";
    if (!convs.length) {
      const e = document.createElement("div");
      e.className = "chat-empty-list";
      e.textContent = t("chatNoConvs");
      box.appendChild(e);
      return;
    }
    for (const c of convs) {
      const row = document.createElement("div");
      row.className = "conv" + (c.id === activeId ? " active" : "");
      const ct = document.createElement("span");
      ct.className = "ct";
      ct.textContent = c.title || t("chatUntitled");
      const del = document.createElement("span");
      del.className = "del";
      del.textContent = "✕";
      del.title = t("chatDelete");
      del.onclick = (ev) => {
        ev.stopPropagation();
        if (!confirm(t("chatConfirmDel"))) return;
        convs = convs.filter((x) => x.id !== c.id);
        if (activeId === c.id) activeId = convs.length ? convs[0].id : null;
        saveSoon();
        renderConvs();
        renderMessages();
      };
      row.onclick = () => {
        if (streaming) return;
        activeId = c.id;
        renderConvs();
        renderMessages();
      };
      row.appendChild(ct);
      row.appendChild(del);
      box.appendChild(row);
    }
  }

  function bubble(role) {
    const b = document.createElement("div");
    b.className = "bubble " + (role === "user" ? "user" : "bot");
    return b;
  }

  function renderMessages() {
    const box = $("chat-messages");
    box.innerHTML = "";
    const c = active();
    if (!c || !c.messages.length) {
      const h = document.createElement("div");
      h.className = "chat-hello";
      h.textContent = S && S.llama === "ready" ? t("chatEmpty") : t("chatNoModel");
      box.appendChild(h);
      return;
    }
    for (const m of c.messages) {
      const b = bubble(m.role);
      b.textContent = m.content;
      box.appendChild(b);
    }
    box.scrollTop = box.scrollHeight;
  }

  function setSending(on) {
    streaming = on;
    const btn = $("chat-send");
    btn.textContent = on ? t("chatStop") : t("chatSend");
    $("chat-model").disabled = on;
  }

  async function send() {
    if (streaming) {
      cinv("chat_stop");
      return;
    }
    const input = $("chat-input");
    const text = input.value.trim();
    if (!text) return;
    if (!S || S.llama !== "ready") {
      renderMessages();
      return;
    }
    let c = active();
    if (!c) {
      c = { id: uid(), title: "", messages: [], updated: Date.now() };
      convs.unshift(c);
      activeId = c.id;
    }
    c.messages.push({ role: "user", content: text });
    if (!c.title) c.title = text.slice(0, 40);
    input.value = "";
    input.style.height = "auto";
    renderConvs();
    renderMessages();

    // пузырь бота со стримингом
    const box = $("chat-messages");
    botEl = bubble("bot");
    botEl.innerHTML =
      '<details class="think" style="display:none"><summary></summary><div class="think-body"></div></details>' +
      '<span class="content cursor"></span>';
    botEl.querySelector("summary").textContent = t("chatThoughts");
    box.appendChild(botEl);
    box.scrollTop = box.scrollHeight;

    curContent = "";
    curThink = "";
    setSending(true);
    try {
      await cinv("chat_send", { messages: c.messages });
    } catch (e) {
      finalize(true, String(e));
    }
  }

  function onToken(p) {
    if (!streaming || !botEl) return;
    if (p.kind === "reasoning") {
      curThink += p.text;
      const th = botEl.querySelector(".think");
      th.style.display = "";
      if (!curContent) th.open = true;
      botEl.querySelector(".think-body").textContent = curThink;
    } else if (p.kind === "content") {
      if (!curContent) {
        const th = botEl.querySelector(".think");
        th.open = false; // прячем размышления, как только пошёл ответ
      }
      curContent += p.text;
      botEl.querySelector(".content").textContent = curContent;
    }
    const box = $("chat-messages");
    box.scrollTop = box.scrollHeight;
  }

  function finalize(isError, errMsg) {
    const c = active();
    if (botEl) {
      const cs = botEl.querySelector(".content");
      if (cs) cs.classList.remove("cursor");
      if (isError) {
        botEl.querySelector(".content").textContent =
          (curContent ? curContent + "\n\n" : "") + "⚠ " + (errMsg || "");
      }
    }
    if (c && curContent && !isError) {
      c.messages.push({ role: "assistant", content: curContent });
      c.updated = Date.now();
      saveSoon();
    } else if (c && isError && curContent) {
      c.messages.push({ role: "assistant", content: curContent });
      saveSoon();
    }
    botEl = null;
    curContent = "";
    curThink = "";
    setSending(false);
  }

  function updateModelBar() {
    const sel = $("chat-model");
    const downloaded = (S && S.downloaded) || [];
    sel.innerHTML = "";
    for (const f of downloaded) {
      const o = document.createElement("option");
      o.value = f;
      o.textContent = modelName(f);
      if (S.settings && S.settings.modelFile === f) o.selected = true;
      sel.appendChild(o);
    }
    $("chat-model-label").textContent = t("chatModelLabel");
    const eng = $("chat-eng");
    const st = S ? S.llama : "";
    eng.textContent =
      st === "ready" ? "● " + t("engineReady").replace("Движок: ", "").replace("Engine: ", "")
      : st === "loading" ? t("engineLoading")
      : st === "stopped" ? t("engineStopped")
      : st && st.startsWith("error:") ? t("engineError") + st.slice(6)
      : "";
  }

  return {
    async init() {
      if (inited) return;
      inited = true;
      try {
        const data = await cinv("chat_get");
        convs = (data && data.conversations) || [];
      } catch (e) {
        convs = [];
      }
      activeId = convs.length ? convs[0].id : null;

      $("chat-new").onclick = () => {
        if (streaming) return;
        activeId = null;
        renderConvs();
        renderMessages();
        $("chat-input").focus();
      };
      $("chat-send").onclick = send;
      const input = $("chat-input");
      input.oninput = () => {
        input.style.height = "auto";
        input.style.height = Math.min(input.scrollHeight, 140) + "px";
      };
      input.onkeydown = (e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          send();
        }
      };
      $("chat-model").onchange = async (e) => {
        try {
          await cinv("use_model", { file: e.target.value });
        } catch (err) {}
      };

      await clisten("chat-token", (e) => onToken(e.payload));
      await clisten("chat-done", (e) => finalize(false));
    },
    onData() {
      // вызывается из settings.js после обновления S
      updateModelBar();
      $("chat-new").textContent = t("chatNew");
      $("chat-input").placeholder = t("chatPlaceholder");
      if (!streaming) $("chat-send").textContent = t("chatSend");
      renderConvs();
      renderMessages();
    },
    activate() {
      updateModelBar();
      renderConvs();
      renderMessages();
      $("chat-input").focus();
    },
  };
})();

// ВАЖНО: делаем объект доступным как window.Chat — settings.js проверяет `if (window.Chat)`.
// Без этого (const в глобальной области не попадает в window) чат не инициализируется.
window.Chat = Chat;
