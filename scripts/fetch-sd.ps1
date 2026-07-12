# Скачивает свежую Vulkan-сборку stable-diffusion.cpp и кладёт нужные файлы
# в src-tauri/resources/sd (эта папка не хранится в git).
# Движок генерации изображений (sd-server.exe), по образцу fetch-llama.ps1.
# Запуск:  powershell -ExecutionPolicy Bypass -File scripts\fetch-sd.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$dst = Join-Path $root "src-tauri\resources\sd"

Write-Host "Ищу последний релиз stable-diffusion.cpp..."
$rel = Invoke-RestMethod "https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest"
$asset = $rel.assets | Where-Object { $_.name -match "bin-win-vulkan-x64\.zip$" } | Select-Object -First 1
if (-not $asset) { throw "Не нашёл win-vulkan-x64 архив в релизе $($rel.tag_name)" }
Write-Host "Релиз $($rel.tag_name): $($asset.name)"

$tmp = Join-Path $env:TEMP "sd-fetch"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null
$zip = Join-Path $tmp "sd.zip"

Invoke-WebRequest $asset.browser_download_url -OutFile $zip
Expand-Archive $zip -DestinationPath $tmp -Force

New-Item -ItemType Directory -Force $dst | Out-Null
Remove-Item "$dst\*" -Force -ErrorAction SilentlyContinue

# Архив кладёт файлы либо в корень, либо в подпапку — ищем рекурсивно.
$srcDir = (Get-ChildItem -Path $tmp -Recurse -Filter "sd-server.exe" | Select-Object -First 1).DirectoryName
if (-not $srcDir) { throw "sd-server.exe не найден в архиве" }

Copy-Item (Join-Path $srcDir "sd-server.exe") $dst
Copy-Item (Join-Path $srcDir "sd-cli.exe") $dst -ErrorAction SilentlyContinue
Get-ChildItem (Join-Path $srcDir "*.dll") | Copy-Item -Destination $dst

Remove-Item -Recurse -Force $tmp
Write-Host "Готово: $dst"
