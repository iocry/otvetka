# Скачивает свежую Vulkan-сборку llama.cpp и кладёт нужные файлы
# в src-tauri/resources/llama (эта папка не хранится в git).
# Запуск:  powershell -ExecutionPolicy Bypass -File scripts\fetch-llama.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$dst = Join-Path $root "src-tauri\resources\llama"

Write-Host "Ищу последний релиз llama.cpp..."
$rel = Invoke-RestMethod "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest"
$asset = $rel.assets | Where-Object { $_.name -match "bin-win-vulkan-x64\.zip$" } | Select-Object -First 1
if (-not $asset) { throw "Не нашёл win-vulkan-x64 архив в релизе $($rel.tag_name)" }
Write-Host "Релиз $($rel.tag_name): $($asset.name)"

$tmp = Join-Path $env:TEMP "llama-fetch"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null
$zip = Join-Path $tmp "llama.zip"

Invoke-WebRequest $asset.browser_download_url -OutFile $zip
Expand-Archive $zip -DestinationPath $tmp -Force

New-Item -ItemType Directory -Force $dst | Out-Null
Remove-Item "$dst\*" -Force -ErrorAction SilentlyContinue

Copy-Item (Join-Path $tmp "llama-server.exe") $dst
Copy-Item (Join-Path $tmp "llama-server-impl.dll") $dst -ErrorAction SilentlyContinue
Copy-Item (Join-Path $tmp "llama.dll") $dst
Copy-Item (Join-Path $tmp "llama-common.dll") $dst -ErrorAction SilentlyContinue
Copy-Item (Join-Path $tmp "mtmd.dll") $dst -ErrorAction SilentlyContinue
Copy-Item (Join-Path $tmp "libomp140.x86_64.dll") $dst -ErrorAction SilentlyContinue
Get-ChildItem (Join-Path $tmp "ggml*.dll") | Where-Object { $_.Name -ne "ggml-rpc.dll" } | Copy-Item -Destination $dst

Remove-Item -Recurse -Force $tmp
Write-Host "Готово: $dst"
