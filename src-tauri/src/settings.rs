use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Style {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: String,
    pub ui_lang: String,
    pub theme: String,
    pub autostart: bool,
    pub model_file: Option<String>,
    pub active_style: String,
    pub custom_styles: Vec<Style>,
    /// Имя пользователя, как он подписан в чатах (для распознавания своих реплик в диалоге)
    pub user_name: String,
    /// Дополнительные имена пользователя в других мессенджерах
    pub user_aliases: Vec<String>,
    /// Скрытые (выключенные) стили — как встроенные, так и свои
    pub hidden_styles: Vec<String>,
    /// Галка «отвечать коротко» в окне вариантов
    pub reply_short: bool,
    /// Автоматически копировать выделенный текст по горячей клавише (эмуляция Ctrl+C)
    pub auto_copy: bool,
    /// Примеры собственных сообщений пользователя — чтобы модель писала «как он»
    pub my_examples: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Shift+R".into(),
            ui_lang: "ru".into(),
            theme: "system".into(),
            autostart: false,
            model_file: None,
            active_style: "friendly".into(),
            custom_styles: vec![],
            user_name: String::new(),
            user_aliases: vec![],
            hidden_styles: vec![],
            reply_short: false,
            auto_copy: true,
            my_examples: String::new(),
        }
    }
}

pub fn path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .expect("no config dir")
        .join("settings.json")
}

/// Возвращает (настройки, это_первый_запуск)
pub fn load(app: &AppHandle) -> (Settings, bool) {
    match std::fs::read_to_string(path(app)) {
        // trim BOM: файл мог быть сохранён внешним редактором в UTF-8 с BOM
        Ok(s) => (
            serde_json::from_str(s.trim_start_matches('\u{feff}')).unwrap_or_default(),
            false,
        ),
        Err(_) => (Settings::default(), true),
    }
}

pub fn save(app: &AppHandle, s: &Settings) -> std::io::Result<()> {
    let p = path(app);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(s).unwrap())
}
