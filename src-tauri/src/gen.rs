use futures_util::StreamExt;
use serde_json::json;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

/// Встроенные стили: (id, инструкция для промпта).
/// Отображаемые названия — на стороне фронтенда (i18n).
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "friendly",
        "по-свойски и тепло, на «ты» — так пишут близкому знакомому: легко, живо, без пафоса",
    ),
    (
        "formal",
        "вежливо и по-деловому, на «вы», но живым человеческим языком — без канцелярита и дежурных шаблонов",
    ),
    (
        "humor",
        "с лёгким живым юмором или самоиронией по теме; смешно, но не натужно и не обидно",
    ),
    (
        "brief",
        "предельно коротко и по существу, как занятой человек: пара слов — и всё понятно",
    ),
    (
        "flirty",
        "игриво, с лёгким флиртом и интригой; ненавязчиво, без пошлости и перегибов",
    ),
    (
        "decline",
        "мягкий по форме, но однозначный отказ — по-человечески, без оправданий на три абзаца",
    ),
];

pub fn builtin_styles() -> serde_json::Value {
    json!(BUILTIN
        .iter()
        .map(|(id, p)| json!({ "id": id, "prompt": p }))
        .collect::<Vec<_>>())
}

fn system_prompt(
    style: &str,
    user_name: &str,
    aliases: &[String],
    short: bool,
    examples: &str,
) -> String {
    let short_rule = if short {
        "- ОТВЕЧАЙ ОЧЕНЬ КОРОТКО: каждый вариант — одна короткая фраза в несколько слов, как быстрый ответ на бегу.\n"
    } else {
        ""
    };
    let examples = examples.trim();
    let example_rule = if examples.is_empty() {
        String::new()
    } else {
        format!(
            "Вот примеры того, как обычно пишет сам пользователь — перенимай его манеру, длину фраз, лексику и пунктуацию, но НЕ копируй эти примеры дословно:\n<<<\n{examples}\n>>>\n"
        )
    };
    let mut identity = String::new();
    let name = user_name.trim();
    if !name.is_empty() {
        identity.push_str(&format!(
            "Пользователя, от чьего лица ты пишешь ответ, зовут «{name}»."
        ));
        let al: Vec<&str> = aliases
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !al.is_empty() {
            identity.push_str(&format!(
                " В разных чатах он также может быть подписан как: {}.",
                al.join(", ")
            ));
        }
        identity.push_str(
            " Если в скопированном тексте есть строки от этих имён — это прошлые реплики самого пользователя (контекст), отвечать на них не нужно; отвечать нужно собеседнику.\n",
        );
    }
    format!(
        "Ты подбираешь варианты ответа в переписке от лица пользователя — обычного живого человека, а не ассистента.\n\
         Тебе дают одно сообщение, несколько сообщений подряд или кусок диалога (метки времени и имена в ответ не включай).\n\
         {identity}\
         {example_rule}\
         Стиль ответа: {style}.\n\
         Как писать:\n\
         - Так, как реальные люди пишут в мессенджерах: просто, разговорно, живыми словами.\n\
         - КАТЕГОРИЧЕСКИ нельзя звучать как ИИ: никаких «Конечно!», «Отличный вопрос», «Надеюсь, это поможет», «Понимаю тебя», никакого канцелярита, вежливых шаблонов и идеально гладких фраз.\n\
         - Не пересказывай сообщения собеседника и не отвечай «по пунктам» как робот.\n\
         - Уместны разговорные слова («ну», «слушай», «блин», «окей»), лёгкая небрежность, недлинные фразы.\n\
         - Если сообщений от собеседника несколько — каждый вариант должен ОДНИМ цельным ответом закрыть все важные моменты сразу, как сделал бы человек.\n\
         - Отвечай на том же языке, на котором пишет собеседник.\n\
         - Смайлики — только если очень уместны, и не больше одного.\n\
         {short_rule}\
         Выдай ровно 3 РАЗНЫХ варианта, без пояснений, заголовков и кавычек, строго в формате:\n\
         1. первый вариант\n\
         2. второй вариант\n\
         3. третий вариант"
    )
}

fn strip_think(content: &str) -> &str {
    match content.rfind("</think>") {
        Some(i) => content[i + "</think>".len()..].trim_start(),
        None => content,
    }
}

fn clean(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| "\"«»“”'`".contains(c))
        .trim()
        .to_string()
}

fn parse_variants(content: &str) -> Vec<String> {
    let content = strip_think(content);
    let mut vars: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        let numbered = t.len() >= 2
            && t.as_bytes()[0].is_ascii_digit()
            && (t.as_bytes()[1] == b'.' || t.as_bytes()[1] == b')');
        if numbered {
            if let Some(c) = cur.take() {
                if !c.trim().is_empty() {
                    vars.push(clean(&c));
                }
            }
            cur = Some(t[2..].trim_start().to_string());
        } else if let Some(c) = cur.as_mut() {
            if !t.is_empty() {
                c.push(' ');
                c.push_str(t);
            }
        }
    }
    if let Some(c) = cur.take() {
        if !c.trim().is_empty() {
            vars.push(clean(&c));
        }
    }
    // На случай, если модель ответила без нумерации — берём непустые строки
    if vars.is_empty() {
        vars = content
            .lines()
            .map(|l| clean(l))
            .filter(|l| !l.is_empty())
            .collect();
    }
    vars.truncate(3);
    vars
}

/// Проверяет готовность движка и возвращает порт.
fn ensure_ready(state: &State<'_, crate::AppState>) -> Result<u16, String> {
    let inner = state.llama.0.lock().unwrap();
    match &inner.status {
        crate::llama::Status::Ready => Ok(inner.port),
        crate::llama::Status::Loading => Err("LOADING".into()),
        crate::llama::Status::Stopped => Err("NO_MODEL".into()),
        crate::llama::Status::Error(e) => Err(format!("ENGINE:{e}")),
    }
}

/// Собирает системный промпт под выбранный стиль из текущих настроек.
fn build_system(state: &State<'_, crate::AppState>, style_id: &str) -> String {
    let s = state.settings.lock().unwrap();
    let style_prompt = BUILTIN
        .iter()
        .find(|(id, _)| *id == style_id)
        .map(|(_, p)| p.to_string())
        .or_else(|| {
            s.custom_styles
                .iter()
                .find(|st| st.id == style_id)
                .map(|st| st.prompt.clone())
        })
        .unwrap_or_else(|| BUILTIN[0].1.to_string());
    let model_file = s.model_file.clone().unwrap_or_default();
    let mut system = system_prompt(
        &style_prompt,
        &s.user_name,
        &s.user_aliases,
        s.reply_short,
        &s.my_examples,
    );
    // У «думающих» версий Qwen3 (все, кроме instruct-2507) отключаем режим размышлений
    if model_file.contains("Qwen3-") && !model_file.contains("2507") {
        system.push_str("\n/no_think");
    }
    system
}

#[tauri::command]
pub async fn generate(
    _app: AppHandle,
    state: State<'_, crate::AppState>,
    text: String,
    style_id: String,
) -> Result<Vec<String>, String> {
    let port = ensure_ready(&state)?;
    let system = build_system(&state, &style_id);

    let body = json!({
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": text }
        ],
        "temperature": 0.9,
        "top_p": 0.95,
        "max_tokens": 700,
        "stop": ["\n4.", "\n4)"]
    });

    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let resp = state
        .client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("ENGINE:{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("ENGINE:http {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("ENGINE:{e}"))?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    let vars = parse_variants(content);
    if vars.is_empty() {
        return Err("EMPTY".into());
    }
    Ok(vars)
}

/// Прервать текущую потоковую генерацию вариантов.
#[tauri::command]
pub fn generate_stop(state: State<'_, crate::AppState>) {
    state.gen_gen.fetch_add(1, Ordering::SeqCst);
}

/// Потоковая генерация вариантов: шлёт сырой текст событиями `gen-token`,
/// фронтенд сам разбивает его на «1. 2. 3.» на лету.
#[tauri::command]
pub async fn generate_stream(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    text: String,
    style_id: String,
) -> Result<(), String> {
    let port = ensure_ready(&state)?;
    let system = build_system(&state, &style_id);
    let my_gen = state.gen_gen.fetch_add(1, Ordering::SeqCst) + 1;

    let body = json!({
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": text }
        ],
        "temperature": 0.9,
        "top_p": 0.95,
        "max_tokens": 700,
        "stop": ["\n4.", "\n4)"],
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
        if state.gen_gen.load(Ordering::SeqCst) != my_gen {
            return Ok(()); // отменено новой генерацией
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
                if let Some(c) = v["choices"][0]["delta"]["content"].as_str() {
                    if !c.is_empty() {
                        let _ = app.emit("gen-token", json!({ "gen": my_gen, "text": c }));
                    }
                }
            }
        }
    }
    let _ = app.emit("gen-done", json!({ "gen": my_gen }));
    Ok(())
}
