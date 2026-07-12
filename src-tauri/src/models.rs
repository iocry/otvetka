use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

/// Объём оперативной памяти в ГБ (через GlobalMemoryStatusEx).
#[cfg(windows)]
fn total_ram_gb() -> f64 {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page: u64,
        avail_page: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_ext_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(p: *mut MemoryStatusEx) -> i32;
    }
    unsafe {
        let mut m: MemoryStatusEx = std::mem::zeroed();
        m.length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            return m.total_phys as f64 / 1024.0 / 1024.0 / 1024.0;
        }
    }
    0.0
}
#[cfg(not(windows))]
fn total_ram_gb() -> f64 {
    0.0
}

/// Дискретная ли видеокарта (по названию), а не встроенная в процессор.
fn is_discrete_gpu(name: &str) -> bool {
    let u = name.to_uppercase();
    [
        "NVIDIA", "GEFORCE", "RTX", "GTX", "QUADRO", "TESLA", "RADEON RX", "RADEON PRO", "ARC",
    ]
    .iter()
    .any(|k| u.contains(k))
}

/// Определяет видеокарту через `llama-server --list-devices`.
/// Возвращает (имя, видеопамять ГБ, дискретная_ли).
/// Приоритет — дискретной карте: встроенная (AMD APU / Intel) показывает
/// разделяемую системную память и вводит в заблуждение.
fn detect_gpu(app: &AppHandle) -> (String, f64, bool) {
    let Some(exe) = crate::llama::server_exe(app) else {
        return (String::new(), 0.0, false);
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--list-devices");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(out) = cmd.output() else {
        return (String::new(), 0.0, false);
    };
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    // строки вида: "  Vulkan0: NVIDIA GeForce RTX 5070 Ti (15995 MiB, 15227 MiB free)"
    let mut devices: Vec<(String, u64)> = Vec::new();
    for line in text.lines() {
        let Some(op) = line.find('(') else { continue };
        let after = &line[op + 1..];
        let Some(mib_pos) = after.find(" MiB") else { continue };
        let num: String = after[..mib_pos]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        let Ok(n) = num.parse::<u64>() else { continue };
        let name = match line.find(": ") {
            Some(colon) => line[colon + 2..op].trim().to_string(),
            None => line[..op].trim().to_string(),
        };
        if !name.is_empty() {
            devices.push((name, n));
        }
    }
    if devices.is_empty() {
        return (String::new(), 0.0, false);
    }

    // сначала ищем дискретную карту; если нет — берём с наибольшей памятью
    let chosen = devices
        .iter()
        .find(|(n, _)| is_discrete_gpu(n))
        .or_else(|| devices.iter().max_by_key(|(_, m)| *m))
        .cloned()
        .unwrap();
    let discrete = is_discrete_gpu(&chosen.0);
    (chosen.0, chosen.1 as f64 / 1024.0, discrete)
}

/// Рекомендация модели под железо пользователя.
#[tauri::command]
pub fn recommend_model(app: AppHandle) -> serde_json::Value {
    let ram_gb = total_ram_gb();
    let (gpu, vram_gb, discrete) = detect_gpu(&app);
    // На дискретной видеокарте ориентируемся на её видеопамять; без неё —
    // на объём ОЗУ (встроенная графика тянет тяжёлые модели слабо).
    let rec = if discrete && vram_gb >= 11.0 {
        "big14"
    } else if discrete && vram_gb >= 7.0 {
        "big8"
    } else if discrete && vram_gb >= 4.0 {
        "standard"
    } else if discrete && vram_gb >= 1.0 {
        "light"
    } else if ram_gb >= 16.0 {
        "standard"
    } else {
        "light"
    };
    json!({
        "ramGb": (ram_gb * 10.0).round() / 10.0,
        "vramGb": (vram_gb * 10.0).round() / 10.0,
        "gpu": gpu,
        "discrete": discrete,
        "recommendedId": rec,
    })
}

pub struct ModelDef {
    pub id: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size_mb: u64,
}

pub const CATALOG: &[ModelDef] = &[
    ModelDef {
        id: "standard",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size_mb: 2440,
    },
    ModelDef {
        id: "light",
        file: "Qwen3-1.7B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        size_mb: 1070,
    },
    ModelDef {
        id: "big8",
        file: "Qwen3-8B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
        size_mb: 4795,
    },
    ModelDef {
        id: "big14",
        file: "Qwen3-14B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf",
        size_mb: 8585,
    },
    // Расцензуренная (abliterated) версия стандартной модели — для улучшения
    // промтов к картинкам без отказов (в т.ч. откровенных). Тоже полноценная
    // текстовая модель: её можно выбрать активной и для ответов/чата.
    ModelDef {
        id: "uncensored",
        file: "Huihui-Qwen3-4B-Instruct-2507-abliterated.Q4_K_M.gguf",
        url: "https://huggingface.co/mradermacher/Huihui-Qwen3-4B-Instruct-2507-abliterated-GGUF/resolve/main/Huihui-Qwen3-4B-Instruct-2507-abliterated.Q4_K_M.gguf",
        size_mb: 2381,
    },
];

pub fn catalog_json() -> serde_json::Value {
    json!(CATALOG
        .iter()
        .map(|m| json!({ "id": m.id, "file": m.file, "sizeMb": m.size_mb }))
        .collect::<Vec<_>>())
}

pub fn downloaded_files(app: &AppHandle) -> Vec<String> {
    let dir = crate::llama::models_dir(app);
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".gguf"))
                .collect()
        })
        .unwrap_or_default()
}

fn activate(app: &AppHandle, state: &State<'_, crate::AppState>, file: &str) -> Result<(), String> {
    let snapshot = {
        let mut s = state.settings.lock().unwrap();
        s.model_file = Some(file.to_string());
        s.clone()
    };
    crate::settings::save(app, &snapshot).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", &snapshot);
    crate::llama::restart(app.clone());
    Ok(())
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let def = CATALOG
        .iter()
        .find(|m| m.id == id)
        .ok_or("unknown model")?;
    let dir = crate::llama::models_dir(&app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let final_path = dir.join(def.file);
    if final_path.exists() {
        return activate(&app, &state, def.file);
    }

    let cancel = state.dl_cancel.clone();
    cancel.store(false, Ordering::SeqCst);

    let part = dir.join(format!("{}.part", def.file));
    let resp = state
        .client
        .get(def.url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("http {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(def.size_mb * 1024 * 1024);

    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    use futures_util::StreamExt;
    use std::io::Write;
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            let _ = app.emit(
                "model-dl-progress",
                json!({ "id": id, "downloaded": 0, "total": total, "done": false, "canceled": true }),
            );
            return Err("CANCELED".into());
        }
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emit > 3 * 1024 * 1024 {
            last_emit = downloaded;
            let _ = app.emit(
                "model-dl-progress",
                json!({ "id": id, "downloaded": downloaded, "total": total, "done": false }),
            );
        }
    }
    drop(file);
    std::fs::rename(&part, &final_path).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "model-dl-progress",
        json!({ "id": id, "downloaded": downloaded, "total": total, "done": true }),
    );
    activate(&app, &state, def.file)
}

#[tauri::command]
pub fn cancel_download(state: State<'_, crate::AppState>) {
    state.dl_cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn use_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    file: String,
) -> Result<(), String> {
    if file.contains('/') || file.contains('\\') {
        return Err("bad name".into());
    }
    if !crate::llama::models_dir(&app).join(&file).exists() {
        return Err("not downloaded".into());
    }
    activate(&app, &state, &file)
}

#[tauri::command]
pub fn delete_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    file: String,
) -> Result<(), String> {
    if file.contains('/') || file.contains('\\') {
        return Err("bad name".into());
    }
    let active = state.settings.lock().unwrap().model_file.clone();
    if active.as_deref() == Some(file.as_str()) {
        // Модель активна: останавливаем движок (он держит файл открытым) и снимаем выбор
        crate::llama::kill(&app);
        let snapshot = {
            let mut s = state.settings.lock().unwrap();
            s.model_file = None;
            s.clone()
        };
        crate::settings::save(&app, &snapshot).map_err(|e| e.to_string())?;
        let _ = app.emit("settings-changed", &snapshot);
        crate::llama::emit_status(&app);
    }
    std::fs::remove_file(crate::llama::models_dir(&app).join(&file)).map_err(|e| e.to_string())
}

// ============================================================================
//  Модели генерации изображений (stable-diffusion.cpp)
// ============================================================================

/// Отдельный файл модели изображения. `role`:
/// "model" (единый чекпоинт SDXL) | "diffusion" | "t5xxl" | "vae".
pub struct ImageFile {
    pub role: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size_mb: u64,
}

pub struct ImageModelDef {
    pub id: &'static str,
    /// "sdxl" (единый файл) | "chroma" (набор файлов)
    pub kind: &'static str,
    pub files: &'static [ImageFile],
    /// Разумные значения по умолчанию для UI
    pub steps: u32,
    pub cfg: f32,
}

pub const IMAGE_CATALOG: &[ImageModelDef] = &[
    ImageModelDef {
        id: "sdxl",
        kind: "sdxl",
        steps: 26,
        cfg: 6.0,
        files: &[ImageFile {
            role: "model",
            file: "sdxlYamersRealisticNSFW_v5TX.safetensors",
            url: "https://huggingface.co/misri/sdxlYamersRealisticNSFW_v5TX/resolve/main/sdxlYamersRealisticNSFW_v5TX.safetensors",
            size_mb: 6617,
        }],
    },
    ImageModelDef {
        id: "chroma",
        kind: "chroma",
        steps: 26,
        cfg: 4.0,
        files: &[
            ImageFile {
                role: "diffusion",
                file: "Chroma1-HD-Q8_0.gguf",
                url: "https://huggingface.co/silveroxides/Chroma-GGUF/resolve/main/Chroma1-HD/Chroma1-HD-Q8_0.gguf",
                size_mb: 9285,
            },
            ImageFile {
                role: "t5xxl",
                file: "t5-v1_1-xxl-encoder-Q4_K_M.gguf",
                url: "https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf/resolve/main/t5-v1_1-xxl-encoder-Q4_K_M.gguf",
                size_mb: 2762,
            },
            ImageFile {
                role: "vae",
                file: "ae.safetensors",
                url: "https://huggingface.co/Comfy-Org/Lumina_Image_2.0_Repackaged/resolve/main/split_files/vae/ae.safetensors",
                size_mb: 320,
            },
        ],
    },
    // Редактор фото: FLUX.1 Kontext — правит прикреплённое фото по инструкции
    // («перекрась куртку в красный»), сохраняя лицо и композицию.
    // T5 и VAE общие с Chroma — если она скачана, докачиваются только
    // сама модель и clip_l.
    ImageModelDef {
        id: "kontext",
        kind: "kontext",
        steps: 22,
        cfg: 1.0,
        files: &[
            ImageFile {
                role: "diffusion",
                file: "flux1-kontext-dev-Q6_K.gguf",
                url: "https://huggingface.co/QuantStack/FLUX.1-Kontext-dev-GGUF/resolve/main/flux1-kontext-dev-Q6_K.gguf",
                size_mb: 9392,
            },
            ImageFile {
                role: "clip_l",
                file: "clip_l.safetensors",
                url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors",
                size_mb: 234,
            },
            ImageFile {
                role: "t5xxl",
                file: "t5-v1_1-xxl-encoder-Q4_K_M.gguf",
                url: "https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf/resolve/main/t5-v1_1-xxl-encoder-Q4_K_M.gguf",
                size_mb: 2762,
            },
            ImageFile {
                role: "vae",
                file: "ae.safetensors",
                url: "https://huggingface.co/Comfy-Org/Lumina_Image_2.0_Repackaged/resolve/main/split_files/vae/ae.safetensors",
                size_mb: 320,
            },
        ],
    },
];

pub fn image_models_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .expect("no data dir")
        .join("models-image")
}

fn image_def(id: &str) -> Option<&'static ImageModelDef> {
    IMAGE_CATALOG.iter().find(|m| m.id == id)
}

fn image_total_mb(def: &ImageModelDef) -> u64 {
    def.files.iter().map(|f| f.size_mb).sum()
}

/// Все ли файлы модели скачаны.
fn image_complete(app: &AppHandle, def: &ImageModelDef) -> bool {
    let dir = image_models_dir(app);
    def.files.iter().all(|f| dir.join(f.file).exists())
}

pub fn image_catalog_json() -> serde_json::Value {
    json!(IMAGE_CATALOG
        .iter()
        .map(|m| json!({
            "id": m.id,
            "kind": m.kind,
            "sizeMb": image_total_mb(m),
            "steps": m.steps,
            "cfg": m.cfg,
        }))
        .collect::<Vec<_>>())
}

/// id моделей, у которых скачаны ВСЕ файлы.
pub fn downloaded_image_ids(app: &AppHandle) -> Vec<String> {
    IMAGE_CATALOG
        .iter()
        .filter(|m| image_complete(app, m))
        .map(|m| m.id.to_string())
        .collect()
}

/// Рекомендация image-модели под железо (SD-модели прожорливее по VRAM).
#[tauri::command]
pub fn recommend_image_model(app: AppHandle) -> serde_json::Value {
    let ram_gb = total_ram_gb();
    let (gpu, vram_gb, discrete) = detect_gpu(&app);
    let rec = if discrete && vram_gb >= 12.0 {
        "chroma"
    } else {
        "sdxl"
    };
    json!({
        "ramGb": (ram_gb * 10.0).round() / 10.0,
        "vramGb": (vram_gb * 10.0).round() / 10.0,
        "gpu": gpu,
        "discrete": discrete,
        "recommendedId": rec,
    })
}

fn activate_image(app: &AppHandle, state: &State<'_, crate::AppState>, id: &str) -> Result<(), String> {
    let snapshot = {
        let mut s = state.settings.lock().unwrap();
        s.image_model = Some(id.to_string());
        s.clone()
    };
    crate::settings::save(app, &snapshot).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-changed", &snapshot);
    // Ленивая загрузка: не грузим новую модель сразу, только выгружаем старую
    // (освобождаем VRAM). Новая загрузится при первой генерации.
    crate::image::unload(app);
    Ok(())
}

/// Скачивает все файлы image-модели последовательно с общим прогрессом.
#[tauri::command]
pub async fn download_image_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let def = image_def(&id).ok_or("unknown model")?;
    let dir = image_models_dir(&app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let cancel = state.img_dl_cancel.clone();
    cancel.store(false, Ordering::SeqCst);

    let grand_total: u64 = def.files.iter().map(|f| f.size_mb * 1024 * 1024).sum();
    let mut done_bytes: u64 = 0; // байты уже полностью скачанных файлов

    use futures_util::StreamExt;
    use std::io::Write;

    for f in def.files {
        let final_path = dir.join(f.file);
        if final_path.exists() {
            done_bytes += f.size_mb * 1024 * 1024;
            continue;
        }
        let part = dir.join(format!("{}.part", f.file));
        let resp = state
            .client
            .get(f.url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("http {}", resp.status()));
        }
        let file_total = resp.content_length().unwrap_or(f.size_mb * 1024 * 1024);

        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut stream = resp.bytes_stream();
        let mut file_dl: u64 = 0;
        let mut last_emit: u64 = 0;

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&part);
                let _ = app.emit(
                    "image-dl-progress",
                    json!({ "id": id, "downloaded": 0, "total": grand_total, "done": false, "canceled": true }),
                );
                return Err("CANCELED".into());
            }
            let chunk = chunk.map_err(|e| e.to_string())?;
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            file_dl += chunk.len() as u64;
            let overall = done_bytes + file_dl;
            if overall - last_emit > 3 * 1024 * 1024 {
                last_emit = overall;
                let _ = app.emit(
                    "image-dl-progress",
                    json!({ "id": id, "downloaded": overall, "total": grand_total, "done": false }),
                );
            }
        }
        drop(file);
        std::fs::rename(&part, &final_path).map_err(|e| e.to_string())?;
        done_bytes += file_total.max(file_dl);
    }

    let _ = app.emit(
        "image-dl-progress",
        json!({ "id": id, "downloaded": grand_total, "total": grand_total, "done": true }),
    );
    activate_image(&app, &state, &id)
}

#[tauri::command]
pub fn cancel_image_download(state: State<'_, crate::AppState>) {
    state.img_dl_cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn use_image_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let def = image_def(&id).ok_or("unknown model")?;
    if !image_complete(&app, def) {
        return Err("not downloaded".into());
    }
    activate_image(&app, &state, &id)
}

#[tauri::command]
pub fn delete_image_model(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let def = image_def(&id).ok_or("unknown model")?;
    let active = state.settings.lock().unwrap().image_model.clone();
    if active.as_deref() == Some(id.as_str()) {
        // Модель активна: гасим движок (он держит файлы открытыми) и снимаем выбор
        crate::image::kill(&app);
        let snapshot = {
            let mut s = state.settings.lock().unwrap();
            s.image_model = None;
            s.clone()
        };
        crate::settings::save(&app, &snapshot).map_err(|e| e.to_string())?;
        let _ = app.emit("settings-changed", &snapshot);
        crate::image::emit_status(&app);
    }
    let dir = image_models_dir(&app);
    for f in def.files {
        let _ = std::fs::remove_file(dir.join(f.file));
    }
    Ok(())
}
