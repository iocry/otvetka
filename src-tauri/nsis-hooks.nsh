; Хуки установщика NSIS для «Ответки».
; Задача: при НАСТОЯЩЕМ удалении программы (не при обновлении) предложить
; удалить скачанные модели и настройки, чтобы после себя ничего не оставалось.
;
; $UpdateMode = 1 означает, что деинсталлятор запущен движком обновления Tauri
; (с флагом /UPDATE) — в этом случае данные НЕ трогаем, модели сохраняются.
; ${Silent} — тихий режим, тоже не спрашиваем.

!include "LogicLib.nsh"

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIfNot} ${Silent}
    MessageBox MB_YESNO|MB_ICONQUESTION "Удалить также скачанные модели ИИ и настройки?$\n(Delete downloaded AI models and settings too?)$\n$\nНажмите «Нет», если собираетесь переустановить приложение и хотите сохранить модели." IDNO otvetka_keep_data
      RMDir /r "$APPDATA\ru.otvetka.app"
      RMDir /r "$LOCALAPPDATA\ru.otvetka.app"
    otvetka_keep_data:
  ${EndIf}
!macroend
