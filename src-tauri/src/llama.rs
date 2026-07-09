use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default, Clone, PartialEq)]
pub enum Status {
    #[default]
    Stopped,
    Loading,
    Ready,
    Error(String),
}

#[derive(Default)]
pub struct Inner {
    pub child: Option<Child>,
    pub port: u16,
    pub status: Status,
    pub generation: u64,
}

#[derive(Default)]
pub struct LlamaState(pub Arc<Mutex<Inner>>);

impl LlamaState {
    pub fn status_string(&self) -> String {
        match &self.0.lock().unwrap().status {
            Status::Stopped => "stopped".into(),
            Status::Loading => "loading".into(),
            Status::Ready => "ready".into(),
            Status::Error(e) => format!("error:{e}"),
        }
    }
}

pub fn models_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path().app_data_dir().expect("no data dir").join("models")
}

pub(crate) fn server_exe(app: &AppHandle) -> Option<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = vec![];
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("llama").join("llama-server.exe"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("llama").join("llama-server.exe"));
        }
    }
    // dev-запуск из корня проекта
    candidates.push(std::path::PathBuf::from(
        "src-tauri/resources/llama/llama-server.exe",
    ));
    candidates.into_iter().find(|p| p.exists())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(18723)
}

pub fn kill(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    let mut inner = state.llama.0.lock().unwrap();
    inner.generation += 1;
    if let Some(mut c) = inner.child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    inner.status = Status::Stopped;
}

fn set_error(app: &AppHandle, msg: &str) {
    let state = app.state::<crate::AppState>();
    state.llama.0.lock().unwrap().status = Status::Error(msg.into());
    emit_status(app);
}

pub fn emit_status(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    let s = state.llama.status_string();
    let _ = app.emit("llama-status", serde_json::json!({ "status": s }));
}

/// Перезапускает llama-server с моделью из настроек (или останавливает, если модели нет).
pub fn restart(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::AppState>();
        let model_file = state.settings.lock().unwrap().model_file.clone();

        {
            let mut inner = state.llama.0.lock().unwrap();
            inner.generation += 1;
            if let Some(mut c) = inner.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            inner.status = Status::Stopped;
        }

        let Some(model_file) = model_file else {
            emit_status(&app);
            return;
        };
        let model_path = models_dir(&app).join(&model_file);
        if !model_path.exists() {
            set_error(&app, "model_missing");
            return;
        }
        let Some(exe) = server_exe(&app) else {
            set_error(&app, "engine_missing");
            return;
        };

        let port = free_port();
        let log = app
            .path()
            .app_data_dir()
            .ok()
            .and_then(|d| {
                let _ = std::fs::create_dir_all(&d);
                std::fs::File::create(d.join("llama-server.log")).ok()
            });

        let mut cmd = Command::new(&exe);
        cmd.arg("-m")
            .arg(&model_path)
            .args([
                "--host", "127.0.0.1",
                "--port", &port.to_string(),
                "-c", "4096",
                "-ngl", "99",
                "--jinja",
                "--no-webui",
            ])
            .stdout(Stdio::null());
        match log {
            Some(f) => { cmd.stderr(Stdio::from(f)); }
            None => { cmd.stderr(Stdio::null()); }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_error(&app, &format!("spawn: {e}"));
                return;
            }
        };

        let my_gen;
        {
            let mut inner = state.llama.0.lock().unwrap();
            inner.child = Some(child);
            inner.port = port;
            inner.status = Status::Loading;
            my_gen = inner.generation;
        }
        emit_status(&app);

        // Ждём, пока сервер прогрузит модель
        let client = state.client.clone();
        let url = format!("http://127.0.0.1:{port}/health");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            {
                let mut inner = state.llama.0.lock().unwrap();
                if inner.generation != my_gen {
                    return;
                }
                if let Some(c) = inner.child.as_mut() {
                    if let Ok(Some(st)) = c.try_wait() {
                        inner.status = Status::Error(format!("exited: {st}"));
                        drop(inner);
                        emit_status(&app);
                        return;
                    }
                }
            }
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    let mut inner = state.llama.0.lock().unwrap();
                    if inner.generation != my_gen {
                        return;
                    }
                    inner.status = Status::Ready;
                    drop(inner);
                    emit_status(&app);
                    return;
                }
            }
            if std::time::Instant::now() > deadline {
                set_error(&app, "load_timeout");
                return;
            }
        }
    });
}
