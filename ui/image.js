// Вкладка «Картинки» — локальная генерация изображений (stable-diffusion.cpp).
// Управление МОДЕЛЯМИ вынесено на вкладку «Модель» (settings.js). Здесь — только
// сама генерация: промт, настройки, улучшение промта, оценка времени, галерея.
// Опирается на глобальные t / lang / S / IMAGE_MODEL_META / MODEL_META из settings.js.

const ImageGen = (function () {
  const iinv = window.__TAURI__ ? window.__TAURI__.core.invoke : async () => {};
  const $ = (id) => document.getElementById(id);

  let inited = false;
  let generating = false;
  let gallery = []; // [{ file, prompt }]
  const urlCache = {}; // file -> data URL
  let pending = 0; // сколько картинок сейчас генерится (заглушки)
  let warmedUp = false; // первая генерация после старта — с прогревом (медленнее)

  // Параметры генерации
  let params = { size: "1024x1024", count: 1, steps: 26, cfg: 6, seedRand: true, seed: 0, strength: 0.6 };
  let paramModelId = null;
  let initImage = null; // data-URL фото-референса (img2img)

  // Калибровка времени: мс на (шаг × мегапиксель × картинку) по типу модели.
  // Значения из реального замера на RTX 5070 Ti (Vulkan): SDXL ~быстро,
  // Chroma ~2.7 с/шаг при 1024². Дальше уточняется по факту (localStorage).
  const DEFAULT_CALIB = { sdxl: 500, chroma: 3200, kontext: 3200 };
  let calib = {};
  try { calib = JSON.parse(localStorage.getItem("imgCalib") || "{}"); } catch (e) { calib = {}; }

  // Живой отсчёт времени во время генерации (чтобы было видно, что процесс идёт).
  let genTimer = null;
  function startGenTicker() {
    const def = activeDef();
    const [w, h] = params.size.split("x").map(Number);
    const mp = (w * h) / 1048576;
    const per = (def && (calib[def.kind] || DEFAULT_CALIB[def.kind])) || 1000;
    const estS = Math.round((per * params.steps * mp * params.count) / 1000);
    const start = Date.now();
    clearInterval(genTimer);
    const tick = () => {
      const el = $("img-estimate");
      if (!el) return;
      const s = Math.round((Date.now() - start) / 1000);
      el.textContent = "⏳ " + s + " " + t("secPerImg") + " / ~" + estS + " " + t("secPerImg") + " …";
    };
    tick();
    genTimer = setInterval(tick, 1000);
  }
  function stopGenTicker() {
    clearInterval(genTimer);
    genTimer = null;
    updateEstimate();
  }

  function activeDef() {
    if (!S) return null;
    return (S.imageCatalog || []).find((m) => m.id === S.settings.imageModel) || null;
  }

  function textModelName(file) {
    const m = (S.modelCatalog || []).find((x) => x.file === file);
    if (m && MODEL_META[m.id]) return MODEL_META[m.id].name[lang] || MODEL_META[m.id].name.ru;
    return file.replace(/\.gguf$/i, "");
  }

  // ---------- оценка времени ----------

  function fmtDur(ms) {
    const s = ms / 1000;
    if (s < 90) return "≈" + Math.max(1, Math.round(s)) + " " + t("secPerImg");
    return "≈" + Math.round(s / 60) + (lang === "ru" ? " мин" : " min");
  }

  function updateEstimate() {
    const el = $("img-estimate");
    if (!el) return;
    const def = activeDef();
    if (!def) { el.textContent = ""; return; }
    const [w, h] = params.size.split("x").map(Number);
    const mp = (w * h) / 1048576;
    const per = calib[def.kind] || DEFAULT_CALIB[def.kind] || 300;
    const ms = per * params.steps * mp * params.count;
    let s = t("imgEstimate") + ": " + fmtDur(ms);
    if (!warmedUp) s += " · " + t("imgEstFirst");
    el.textContent = s;
  }

  function calibrate(ms, count, w, h) {
    const def = activeDef();
    if (!def) return;
    const mp = (w * h) / 1048576;
    const per = ms / (params.steps * mp * count);
    // Первую (прогревочную) генерацию в калибровку не берём, чтобы не завышать оценку.
    if (warmedUp && per > 0 && isFinite(per)) {
      calib[def.kind] = (calib[def.kind] || per) * 0.5 + per * 0.5;
      try { localStorage.setItem("imgCalib", JSON.stringify(calib)); } catch (e) {}
    }
    warmedUp = true;
  }

  // ---------- форма ----------

  function engineText() {
    const st = S ? S.image : "";
    if (st === "ready") return t("imgReady");
    if (st === "loading") return t("imgLoadingModel");
    if (st === "stopped") return activeDef() ? t("imgIdle") : t("engineStopped");
    if (st && st.startsWith("error:")) return t("engineError") + st.slice(6);
    return "";
  }

  function syncDefaults() {
    const d = activeDef();
    if (d && paramModelId !== d.id) {
      params.steps = d.steps;
      params.cfg = d.cfg;
      paramModelId = d.id;
    }
    applyParamsToUI();
  }

  function applyParamsToUI() {
    $("img-size").value = params.size;
    $("img-count").value = String(params.count);
    $("img-steps").value = params.steps;
    $("img-steps-val").textContent = params.steps;
    $("img-cfg").value = params.cfg;
    $("img-cfg-val").textContent = params.cfg;
    $("img-seed-rand").checked = params.seedRand;
    $("img-seed").disabled = params.seedRand;
    if (!params.seedRand) $("img-seed").value = params.seed;
    $("img-strength").value = params.strength;
    updateEstimate();
  }

  function uncensoredFile() {
    const m = ((S && S.modelCatalog) || []).find((x) => x.id === "uncensored");
    return m ? m.file : null;
  }
  // Улучшение промта доступно только при скачанной расцензуренной модели.
  function updateEnhanceUI() {
    const f = uncensoredFile();
    const has = !!f && (((S && S.downloaded) || []).includes(f));
    const cb = $("img-enhance");
    cb.disabled = !has;
    if (!has) cb.checked = false;
    $("l-img-enhance-hint").textContent = has ? t("imgEnhanceHint") : t("imgEnhanceNeedModel");
  }

  function renderForm() {
    $("l-img-prompt").textContent = t("imgPromptLabel");
    $("img-prompt").placeholder = t("imgPromptPh");
    $("l-img-prompt-hint").textContent = t("imgPromptRuHint");
    $("l-img-ref").textContent = t("imgRefLabel");
    $("img-ref-btn").textContent = "📎 " + t("imgAttach");
    $("l-img-strength").textContent = t("imgStrength");
    $("img-strength-val").textContent = params.strength;
    $("l-img-strength-hint").textContent = t("imgStrengthHint");
    $("l-img-enhance").textContent = t("imgEnhance");
    // Хинт улучшения промта выставляется в updateEnhanceUI (зависит от наличия модели).
    $("l-img-size").textContent = t("imgSizeLabel");
    const so = $("img-size").options;
    so[0].text = t("imgSizeSquare") + " · 1024×1024";
    so[1].text = t("imgSizePortrait") + " · 832×1216";
    so[2].text = t("imgSizeLandscape") + " · 1216×832";
    so[3].text = "Full HD · 1920×1088";
    so[4].text = "Full HD · 1088×1920";
    $("l-img-size-hint").textContent = t("imgSizeHint");
    $("l-img-count").textContent = t("imgCountLabel");
    $("l-img-adv").textContent = t("imgAdvanced");
    $("l-img-neg").textContent = t("imgNegLabel");
    $("img-neg").placeholder = t("imgNegPh");
    $("l-img-neg-hint").textContent = t("imgNegHint");
    $("l-img-steps").textContent = t("imgSteps");
    $("l-img-steps-hint").textContent = t("imgStepsHint");
    $("l-img-cfg").textContent = t("imgCfg");
    $("l-img-cfg-hint").textContent = t("imgCfgHint");
    $("l-img-seed").textContent = t("imgSeedLabel");
    $("l-img-seed-rand").textContent = t("imgSeedRandom");
    $("l-img-seed-hint").textContent = t("imgSeedHint");
    $("img-reset").textContent = t("imgReset");
    $("img-folder").textContent = "📁 " + t("imgOpenFolder");

    const def = activeDef();
    $("img-nomodel").style.display = def ? "none" : "";
    $("img-nomodel").textContent = t("imgPickModelTab");

    // Фото-референс: превью/кнопка/сила изменений зависят от состояния.
    // У Kontext силы изменений нет (редактирует по инструкции), а хинт — свой.
    const kind = def ? def.kind : "";
    $("img-ref-preview").style.display = initImage ? "" : "none";
    $("img-ref-btn").style.display = initImage ? "none" : "";
    $("img-strength-row").style.display = initImage && kind !== "kontext" ? "" : "none";
    $("l-img-ref-hint").textContent =
      kind === "kontext" ? t("imgRefHintKontext")
      : initImage ? t("imgUseKontextHint")
      : t("imgRefHint");

    // Модель грузится лениво: форма доступна, если модель выбрана и не идёт
    // отдельная загрузка. Во время генерации форму не гасим — чтобы кнопка
    // «Остановить» оставалась кликабельной.
    const loading = S && S.image === "loading";
    const usable = !!def && (!loading || generating);
    $("img-form").classList.toggle("disabled", !usable);
    const btn = $("img-generate");
    if (generating) {
      btn.textContent = t("imgStop");
      btn.disabled = false;
    } else {
      btn.textContent = t("imgGenerate");
      btn.disabled = !usable;
    }
    updateEnhanceUI();
    updateEstimate();
  }

  async function generate() {
    // Повторный клик во время работы = отмена.
    if (generating) {
      iinv("cancel_image").catch(() => {});
      return;
    }
    const def = activeDef();
    if (!def) return;
    // Kontext — редактор фото: без прикреплённого фото не работает
    if (def.kind === "kontext" && !initImage) {
      alert(t("imgKontextNeedsPhoto"));
      return;
    }
    let prompt = $("img-prompt").value.trim();
    if (!prompt) return;

    generating = true;
    renderForm();

    // Улучшение промта (если включено) — только расцензуренной моделью,
    // движок сам её загрузит по требованию.
    if ($("img-enhance").checked) {
      $("img-generate").textContent = t("imgEnhancing");
      try {
        const improved = await iinv("enhance_prompt", { text: prompt });
        if (improved) { prompt = improved; $("img-prompt").value = improved; }
      } catch (e) {
        const msg = String(e).includes("NO_UNCENSORED")
          ? t("imgEnhanceNeedModel")
          : t("imgEnhanceErr") + e;
        alert(msg);
        generating = false;
        renderForm();
        return;
      }
    } else if (/[а-яё]/i.test(prompt)) {
      // Модели рисуют только по английскому тексту — русский промт
      // автоматически переводим (расцензуренной моделью, чтобы перевод
      // откровенных промтов не съедался отказами).
      $("img-generate").textContent = t("imgTranslating");
      try {
        const tr = await iinv("translate_prompt", { text: prompt });
        if (tr) { prompt = tr; $("img-prompt").value = tr; }
      } catch (e) {
        const msg = String(e).includes("NO_UNCENSORED")
          ? t("imgRuNeedsModel")
          : t("imgEnhanceErr") + e;
        alert(msg);
        generating = false;
        renderForm();
        return;
      }
    }

    const [w, h] = params.size.split("x").map(Number);
    const seed = params.seedRand ? -1 : parseInt($("img-seed").value || "0", 10);
    const count = params.count;

    pending = count;
    renderForm();
    renderGallery();
    startGenTicker();

    try {
      const res = await iinv("generate_image", {
        params: {
          prompt,
          negative: $("img-neg").value.trim(),
          width: w,
          height: h,
          steps: params.steps,
          cfg: params.cfg,
          seed,
          count,
          initImage: initImage,
          strength: params.strength,
        },
      });
      const imgs = (res && res.images) || [];
      for (const it of imgs) {
        urlCache[it.file] = it.url;
        gallery.unshift({ file: it.file, prompt });
      }
      if (res && res.ms) calibrate(res.ms, count, w, h);
      saveGallery();
    } catch (e) {
      if (!String(e).includes("CANCELED")) alert(t("imgErr") + e);
    }
    stopGenTicker();
    generating = false;
    pending = 0;
    renderForm();
    renderGallery();
  }

  // ---------- галерея ----------

  const saveGallery = (() => {
    let tmr = null;
    return () => {
      clearTimeout(tmr);
      tmr = setTimeout(
        () =>
          iinv("gallery_set", {
            data: { items: gallery.map((g) => ({ file: g.file, prompt: g.prompt })) },
          }).catch(() => {}),
        300
      );
    };
  })();

  function openLightbox(src) {
    let lb = $("img-lightbox");
    if (!lb) {
      lb = document.createElement("div");
      lb.id = "img-lightbox";
      lb.className = "img-lightbox";
      lb.innerHTML = "<img />";
      lb.onclick = () => lb.classList.remove("on");
      document.body.appendChild(lb);
    }
    lb.querySelector("img").src = src;
    lb.classList.add("on");
  }

  function makeCell(item) {
    const cell = document.createElement("div");
    cell.className = "img-cell";
    const img = document.createElement("img");
    if (urlCache[item.file]) {
      img.src = urlCache[item.file];
    } else {
      iinv("image_data_url", { file: item.file })
        .then((u) => { urlCache[item.file] = u; img.src = u; })
        .catch(() => {});
    }
    if (item.prompt) img.title = item.prompt;
    img.onclick = () => openLightbox(img.src);
    cell.appendChild(img);

    const tools = document.createElement("div");
    tools.className = "tools";
    const del = document.createElement("button");
    del.textContent = "🗑 " + t("imgDelete");
    del.onclick = async (ev) => {
      ev.stopPropagation();
      gallery = gallery.filter((g) => g.file !== item.file);
      delete urlCache[item.file];
      saveGallery();
      renderGallery();
      try { await iinv("delete_image", { file: item.file }); } catch (e) {}
    };
    tools.appendChild(del);
    cell.appendChild(tools);
    return cell;
  }

  function renderGallery() {
    const box = $("img-gallery");
    if (!box) return;
    box.innerHTML = "";
    if (!pending && !gallery.length) {
      const e = document.createElement("div");
      e.className = "img-empty";
      e.textContent = activeDef() ? t("imgGalleryEmpty") : t("imgPickModelTab");
      box.appendChild(e);
      return;
    }
    for (let i = 0; i < pending; i++) {
      const cell = document.createElement("div");
      cell.className = "img-cell pending";
      cell.innerHTML = '<div style="text-align:center"><div class="spin" style="margin:0 auto 8px"></div>' + t("imgGenerating") + "</div>";
      box.appendChild(cell);
    }
    for (const item of gallery) box.appendChild(makeCell(item));
  }

  return {
    async init() {
      if (inited) return;
      inited = true;
      try {
        const data = await iinv("gallery_get");
        gallery = (data && data.items) || [];
      } catch (e) { gallery = []; }

      $("img-generate").onclick = generate;
      $("img-folder").onclick = () => iinv("open_images_dir");
      $("img-reset").onclick = () => {
        paramModelId = null; params.seedRand = true; params.size = "1024x1024"; params.count = 1;
        syncDefaults(); updateEstimate();
      };

      $("img-size").onchange = (e) => { params.size = e.target.value; updateEstimate(); };
      $("img-count").onchange = (e) => { params.count = parseInt(e.target.value, 10); updateEstimate(); };
      $("img-steps").oninput = (e) => { params.steps = parseInt(e.target.value, 10); $("img-steps-val").textContent = params.steps; updateEstimate(); };
      $("img-cfg").oninput = (e) => { params.cfg = parseFloat(e.target.value); $("img-cfg-val").textContent = params.cfg; };
      $("img-seed-rand").onchange = (e) => { params.seedRand = e.target.checked; $("img-seed").disabled = e.target.checked; };
      $("img-seed").onchange = (e) => { params.seed = parseInt(e.target.value || "0", 10); };
      $("img-enhance").onchange = () => updateEnhanceUI();

      // Фото-референс (img2img / Kontext)
      $("img-ref-btn").onclick = () => $("img-ref-file").click();
      $("img-ref-file").onchange = (e) => {
        const f = e.target.files && e.target.files[0];
        if (!f) return;
        const r = new FileReader();
        r.onload = () => {
          initImage = r.result;
          $("img-ref-thumb").src = initImage;
          renderForm();
        };
        r.readAsDataURL(f);
        e.target.value = "";
      };
      $("img-ref-remove").onclick = () => {
        initImage = null;
        renderForm();
      };
      $("img-strength").oninput = (e) => {
        params.strength = parseFloat(e.target.value);
        $("img-strength-val").textContent = params.strength;
      };

      $("img-prompt").onkeydown = (e) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); generate(); }
      };
    },

    onData() {
      if (!S) return;
      $("img-eng").textContent = engineText();
      renderForm();
      syncDefaults();
    },

    activate() {
      this.onData();
      renderGallery();
      const p = $("img-prompt");
      if (p && !$("img-form").classList.contains("disabled")) p.focus();
    },
  };
})();

// Делаем доступным как window.ImageGen — settings.js проверяет `if (window.ImageGen)`.
window.ImageGen = ImageGen;
