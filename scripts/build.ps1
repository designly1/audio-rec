param([switch]$DebugBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$compilerRoot = Join-Path $env:LOCALAPPDATA 'audio-rec\toolchains'
$compiler = Get-ChildItem -LiteralPath $compilerRoot -Directory -ErrorAction SilentlyContinue |
    Sort-Object Name -Descending | Select-Object -First 1
if ($compiler) { $env:PATH = "$($compiler.FullName)\bin;$env:PATH" }
$env:PATH = "$cargoBin;$env:PATH"
Push-Location $projectRoot
try {
    if ($DebugBuild) { cargo build --locked } else { cargo build --release --locked }
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    if (-not $DebugBuild) {
        New-Item -ItemType Directory -Force -Path (Join-Path $projectRoot 'dist') | Out-Null
        Copy-Item -LiteralPath (Join-Path $projectRoot 'target\release\audio-rec.exe') -Destination (Join-Path $projectRoot 'dist\audio-rec.exe')
        Copy-Item -LiteralPath (Join-Path $projectRoot 'README.md') -Destination (Join-Path $projectRoot 'dist\README.md')
        Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $projectRoot 'dist\LICENSE')
        Write-Output 'Ready: dist\audio-rec.exe'
    }
} finally { Pop-Location }

