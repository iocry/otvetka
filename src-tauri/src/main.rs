#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chat;
mod gen;
mod image;
mod llama;
mod models;
mod settings;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    pub settings: Mutex<settings::Settings>,
    pub llama: llama::LlamaState,
    pub image: image::ImageState,
    pub dl_cancel: Arc<AtomicBool>,
    pub img_dl_cancel: Arc<AtomicBool>,
    pub img_cancel: Arc<AtomicBool>,
    pub client: reqwest::Client,
    pub first_run: bool,
    /// Успел ли popup получить фокус после показа — чтобы прятать его
    /// по потере фокуса только если фокус вообще был получен.
    pub popup_focused: AtomicBool,
    /// Поколение генерации в чате — для отмены предыдущего стрима.
    pub chat_gen: AtomicU64,
    /// Поколение потоковой генерации вариантов — для отмены при «ещё варианты».
    pub gen_gen: AtomicU64,
    /// Пользователь сейчас на вкладке генерации картинок (текстовая модель выгружена).
    pub image_mode_active: AtomicBool,
}

/// Эмулирует чистое нажатие Ctrl+C. Перед этим «отпускает» все модификаторы,
/// потому что в момент срабатывания горячей клавиши физически зажаты Ctrl+Shift,
/// и без нейтрализации получилось бы Ctrl+Shift+C вместо Ctrl+C.
#[cfg(windows)]
fn send_ctrl_c() {
    #[link(name = "user32")]
    extern "system" {
        fn keybd_event(b_vk: u8, b_scan: u8, dw_flags: u32, dw_extra: usize);
    }
    const KEYUP: u32 = 0x0002;
    const VK_SHIFT: u8 = 0x10;
    const VK_CONTROL: u8 = 0x11;
    const VK_MENU: u8 = 0x12; // Alt
    const VK_LWIN: u8 = 0x5B;
    const VK_RWIN: u8 = 0x5C;
    const VK_C: u8 = 0x43;
    unsafe {
        // отпускаем все модификаторы, которые мог держать пользователь
        for k in [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, VK_CONTROL] {
            keybd_event(k, 0, KEYUP, 0);
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
        // чистый Ctrl+C
        keybd_event(VK_CONTROL, 0, 0, 0);
        keybd_event(VK_C, 0, 0, 0);
        std::thread::sleep(std::time::Duration::from_millis(15));
        keybd_event(VK_C, 0, KEYUP, 0);
        keybd_event(VK_CONTROL, 0, KEYUP, 0);
    }
}
#[cfg(not(windows))]
fn send_ctrl_c() {}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn position_near_cursor(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Ok(cursor) = app.cursor_position() else {
        return;
    };
    let (w, h) = win
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((420.0, 540.0));
    let mut x = cursor.x - w / 2.0;
    let mut y = cursor.y + 16.0;
    if let Ok(Some(mon)) = app.monitor_from_point(cursor.x, cursor.y) {
        let mp = mon.position();
        let ms = mon.size();
        let (mx, my) = (mp.x as f64, mp.y as f64);
        let (mw, mh) = (ms.width as f64, ms.height as f64);
        if x + w > mx + mw - 8.0 {
            x = mx + mw - w - 8.0;
        }
        if x < mx + 8.0 {
            x = mx + 8.0;
        }
        // не залезать на панель задач
        if y + h > my + mh - 56.0 {
            y = cursor.y - h - 16.0;
        }
        if y < my + 8.0 {
            y = my + 8.0;
        }
    }
    let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
}

fn dbg_log(app: &AppHandle, msg: &str) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("app.log"))
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                "[{:?}] {msg}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
        }
    }
}

fn on_hotkey(app: &AppHandle) {
    dbg_log(app, "hotkey fired");
    // Автокопирование выделенного текста (если включено в настройках)
    let auto_copy = app.state::<AppState>().settings.lock().unwrap().auto_copy;
    if auto_copy {
        send_ctrl_c();
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    let text = arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
        .unwrap_or_default();
    dbg_log(app, &format!("clipboard len={}", text.chars().count()));
    if let Some(win) = app.get_webview_window("popup") {
        app.state::<AppState>()
            .popup_focused
            .store(false, std::sync::atomic::Ordering::SeqCst);
        position_near_cursor(app, &win);
        let show_res = win.show();
        let _ = win.set_focus();
        dbg_log(
            app,
            &format!(
                "popup shown: {show_res:?}, visible={:?}, pos={:?}",
                win.is_visible(),
                win.outer_position()
            ),
        );
        let _ = app.emit_to("popup", "generate-request", json!({ "text": text }));
    } else {
        dbg_log(app, "popup window NOT FOUND");
    }
}

#[tauri::command]
fn get_state(app: AppHandle, state: tauri::State<'_, AppState>) -> serde_json::Value {
    let s = state.settings.lock().unwrap().clone();
    let models_dir = llama::models_dir(&app);
    let total_bytes: u64 = std::fs::read_dir(&models_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0);
    json!({
        "settings": s,
        "builtinStyles": gen::builtin_styles(),
        "modelCatalog": models::catalog_json(),
        "downloaded": models::downloaded_files(&app),
        "llama": state.llama.status_string(),
        "firstRun": state.first_run,
        "version": app.package_info().version.to_string(),
        "modelsDir": models_dir.display().to_string(),
        "modelsBytes": total_bytes,
        "imageCatalog": models::image_catalog_json(),
        "imageDownloaded": models::downloaded_image_ids(&app),
        "image": state.image.status_string(),
    })
}

#[tauri::command]
fn open_models_dir(app: AppHandle) {
    let dir = llama::models_dir(&app);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    new_settings: settings::Settings,
) -> Result<(), String> {
    let old = {
        let mut g = state.settings.lock().unwrap();
        let old = g.clone();
        *g = new_settings.clone();
        old
    };
    settings::save(&app, &new_settings).map_err(|e| e.to_string())?;

    if !old.hotkey.eq_ignore_ascii_case(&new_settings.hotkey) {
        let gs = app.global_shortcut();
        let _ = gs.unregister_all();
        match new_settings.hotkey.parse::<Shortcut>() {
            Ok(sc) => gs.register(sc).map_err(|e| e.to_string())?,
            Err(e) => return Err(format!("HOTKEY:{e}")),
        }
    }
    if old.autostart != new_settings.autostart {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        if new_settings.autostart {
            al.enable().map_err(|e| e.to_string())?;
        } else {
            let _ = al.disable();
        }
    }
    if old.model_file != new_settings.model_file {
        llama::restart(app.clone());
    }
    let _ = app.emit("settings-changed", &new_settings);
    Ok(())
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

#[tauri::command]
fn hide_popup(app: AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.hide();
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        on_hotkey(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_settings,
            copy_text,
            open_settings,
            hide_popup,
            open_models_dir,
            gen::generate,
            gen::generate_stream,
            gen::generate_stop,
            models::download_model,
            models::cancel_download,
            models::use_model,
            models::delete_model,
            models::recommend_model,
            models::download_image_model,
            models::cancel_image_download,
            models::use_image_model,
            models::delete_image_model,
            models::recommend_image_model,
            image::generate_image,
            image::cancel_image,
            image::image_mode,
            image::enhance_prompt,
            image::translate_prompt,
            image::image_data_url,
            image::delete_image,
            image::open_images_dir,
            image::gallery_get,
            image::gallery_set,
            chat::chat_send,
            chat::chat_stop,
            chat::chat_get,
            chat::chat_set,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Миграция данных со старого имени приложения (ru.podskazhi.app → ru.otvetka.app)
            if let Ok(new_dir) = handle.path().app_data_dir() {
                if !new_dir.exists() {
                    if let Some(parent) = new_dir.parent() {
                        let old = parent.join("ru.podskazhi.app");
                        if old.exists() {
                            let _ = std::fs::rename(&old, &new_dir);
                        }
                    }
                }
            }

            let (s, first_run) = settings::load(&handle);
            app.manage(AppState {
                settings: Mutex::new(s.clone()),
                llama: llama::LlamaState::default(),
                image: image::ImageState::default(),
                dl_cancel: Arc::new(AtomicBool::new(false)),
                img_dl_cancel: Arc::new(AtomicBool::new(false)),
                img_cancel: Arc::new(AtomicBool::new(false)),
                client: reqwest::Client::new(),
                first_run,
                popup_focused: AtomicBool::new(false),
                chat_gen: AtomicU64::new(0),
                gen_gen: AtomicU64::new(0),
                image_mode_active: AtomicBool::new(false),
            });

            // Горячая клавиша
            if let Ok(sc) = s.hotkey.parse::<Shortcut>() {
                let _ = app.global_shortcut().register(sc);
            }

            // Трей
            let ru = s.ui_lang == "ru";
            let settings_i = MenuItemBuilder::with_id(
                "settings",
                if ru { "Настройки" } else { "Settings" },
            )
            .build(app)?;
            let quit_i =
                MenuItemBuilder::with_id("quit", if ru { "Выход" } else { "Quit" }).build(app)?;
            let menu = MenuBuilder::new(app).item(&settings_i).item(&quit_i).build()?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Ответка")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "settings" => show_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                        show_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            // Запускаем движок (если модель уже выбрана)
            llama::restart(handle.clone());
            // Движок картинок грузится лениво (при первой генерации); здесь только
            // запускаем монитор простоя, который выгрузит модель через 10 мин без работы.
            image::spawn_idle_monitor(handle.clone());

            // Первый запуск — открываем настройки (там экран выбора модели)
            if first_run || s.model_file.is_none() {
                show_settings(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
                // Закрыли окно настроек (свернули в трей). Если были в режиме
                // картинок — выгружаем картиночную модель из VRAM и возвращаем
                // текстовую (для ответов по хоткею).
                if window.label() == "settings"
                    && window.state::<AppState>().image_mode_active.swap(false, Ordering::SeqCst)
                {
                    image::unload(window.app_handle());
                    llama::restart(window.app_handle().clone());
                }
            }
            WindowEvent::Focused(focused) => {
                if window.label() == "popup" {
                    use std::sync::atomic::Ordering;
                    let flag = &window.state::<AppState>().popup_focused;
                    if *focused {
                        flag.store(true, Ordering::SeqCst);
                    } else if flag.swap(false, Ordering::SeqCst) {
                        // прячем только если окно реально было в фокусе
                        let _ = window.hide();
                    }
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building app")
        .run(|app, event| match event {
            RunEvent::ExitRequested { api, code, .. } => {
                // держим приложение живым в трее, пока не выбран «Выход»
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            RunEvent::Exit => {
                llama::kill(app);
                image::kill(app);
            }
            _ => {}
        });
}
