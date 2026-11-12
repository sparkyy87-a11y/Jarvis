<#
  Baut Mira unter Windows – auf Wunsch mit NVIDIA-Grafikkarte für die Spracherkennung.

    .\scripts\build-windows.ps1            # mit Grafikkarte (CUDA)
    .\scripts\build-windows.ps1 -NurCpu    # ohne Grafikkarte

  Voraussetzungen (einmalig, in dieser Reihenfolge):
    1. Rust:                    https://rustup.rs
    2. Visual Studio Build Tools mit "Desktopentwicklung mit C++"
    3. CMake und LLVM:          winget install Kitware.CMake LLVM.LLVM
    4. Nur für GPU: CUDA Toolkit 12.x (NICHT 13 – das unterstützt GTX 10xx nicht mehr)
       https://developer.nvidia.com/cuda-12-9-0-download-archive
  Danach ein neues Terminal öffnen, damit PATH stimmt.
#>
param(
    [switch]$NurCpu,
    # Rechenarchitektur der Grafikkarte: 61 = GTX 10xx (Pascal), 75 = RTX 20xx, 86 = RTX 30xx, 89 = RTX 40xx
    [string]$CudaArch = "61"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

function Fehlt($was, $wie) { Write-Host "FEHLT: $was – $wie" -ForegroundColor Red; exit 1 }

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Fehlt "Rust" "https://rustup.rs" }
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { Fehlt "CMake" "winget install Kitware.CMake" }
if (-not $env:LIBCLANG_PATH) {
    $llvm = "C:\Program Files\LLVM\bin"
    if (Test-Path "$llvm\libclang.dll") { $env:LIBCLANG_PATH = $llvm } else { Fehlt "LLVM" "winget install LLVM.LLVM" }
}

$features = @()
if (-not $NurCpu) {
    if (-not (Get-Command nvcc -ErrorAction SilentlyContinue)) {
        Fehlt "CUDA Toolkit 12.x" "https://developer.nvidia.com/cuda-12-9-0-download-archive (oder -NurCpu)"
    }
    $version = (nvcc --version | Select-String "release (\d+)\.(\d+)").Matches[0]
    $major = [int]$version.Groups[1].Value
    if ($major -ge 13 -and [int]$CudaArch -lt 75) {
        Fehlt "CUDA 12.x" "CUDA $major unterstützt Pascal-Karten (GTX 10xx) nicht mehr – bitte CUDA 12.x installieren"
    }
    Write-Host "CUDA $($version.Groups[1].Value).$($version.Groups[2].Value), Architektur $CudaArch"
    $env:CMAKE_CUDA_ARCHITECTURES = $CudaArch
    $features = @("--features", "mira-voice/cuda")
}

Write-Host "Baue Mira (das erste Mal dauert einige Minuten) …"
cargo build --release --workspace @features
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$ziel = Join-Path (Get-Location) "dist"
New-Item -ItemType Directory -Force $ziel | Out-Null
foreach ($exe in "mirad.exe", "mira.exe", "mira-listen.exe", "mira-hintergrund.exe", "mira-hud.exe", "mira-einstellungen.exe") {
    Copy-Item "target\release\$exe" $ziel -Force
}
Write-Host ""
Write-Host "Fertig: $ziel" -ForegroundColor Green
Write-Host "  dist\mirad.exe init --name Gaming-PC   (einmalig)"
Write-Host "  dist\mirad.exe autostart an             (startet Mira jetzt und bei jeder Anmeldung)"
