use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

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
