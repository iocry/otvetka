use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

fn chats_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .expect("no data dir")
        .join("chats.json")
}

/// Загрузить сохранённые переписки чата (файл chats.json).
#[tauri::command]
pub fn chat_get(app: AppHandle) -> serde_json::Value {
    match std::fs::read_to_string(chats_path(&app)) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|_| json!({ "conversations": [] })),
        Err(_) => json!({ "conversations": [] }),
    }
}

/// Сохранить переписки чата.
#[tauri::command]
pub fn chat_set(app: AppHandle, data: serde_json::Value) -> Result<(), String> {
    let p = chats_path(&app);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_string(&data).unwrap()).map_err(|e| e.to_string())
}

/// Прервать текущую генерацию в чате.
#[tauri::command]
pub fn chat_stop(state: State<'_, crate::AppState>) {
    state.chat_gen.fetch_add(1, Ordering::SeqCst);
}

/// Отправить сообщения в чат и стримить ответ событиями `chat-token` / `chat-done`.
#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    messages: Vec<ChatMsg>,
) -> Result<(), String> {
    let (status, port) = {
        let i = state.llama.0.lock().unwrap();
        (i.status.clone(), i.port)
    };
    match status {
        crate::llama::Status::Ready => {}
        crate::llama::Status::Loading => return Err("LOADING".into()),
        crate::llama::Status::Stopped => return Err("NO_MODEL".into()),
        crate::llama::Status::Error(e) => return Err(format!("ENGINE:{e}")),
    }

    // Новая генерация отменяет предыдущую (через сравнение поколения)
    let my_gen = state.chat_gen.fetch_add(1, Ordering::SeqCst) + 1;

    let mut msgs = vec![json!({
        "role": "system",
        "content": "Ты — полезный, дружелюбный ассистент. Отвечай понятно и по делу, на том же языке, на котором пишет пользователь. Пиши живым естественным языком."
    })];
    for m in &messages {
        msgs.push(json!({ "role": m.role, "content": m.content }));
    }

    let body = json!({
        "messages": msgs,
        "temperature": 0.7,
        "top_p": 0.95,
        "max_tokens": 2048,
        "stream": true
    });

    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let resp = state
        .client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ENGINE:{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("ENGINE:http {}", resp.status()));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        // отменили или запустили новую генерацию
        if state.chat_gen.load(Ordering::SeqCst) != my_gen {
            let _ = app.emit("chat-done", json!({ "gen": my_gen, "canceled": true }));
            return Ok(());
        }
        let chunk = chunk.map_err(|e| format!("ENGINE:{e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(nl) = buf.find('\n') {
            let line: String = buf.drain(..=nl).collect();
            let line = line.trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" || data.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                let delta = &v["choices"][0]["delta"];
                if let Some(rc) = delta["reasoning_content"].as_str() {
                    if !rc.is_empty() {
                        let _ = app.emit(
                            "chat-token",
                            json!({ "gen": my_gen, "kind": "reasoning", "text": rc }),
                        );
                    }
                }
                if let Some(c) = delta["content"].as_str() {
                    if !c.is_empty() {
                        let _ = app.emit(
                            "chat-token",
                            json!({ "gen": my_gen, "kind": "content", "text": c }),
                        );
                    }
                }
            }
        }
    }

    let _ = app.emit("chat-done", json!({ "gen": my_gen }));
    Ok(())
}
