param([switch]$Offline, [switch]$Release, [switch]$SkipBackend)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $projectRoot
if (!$SkipBackend) {
    & "$PSScriptRoot\build-backend.ps1" -Offline:$Offline
}
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$pnpmTool = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
if ($pnpmTool) { $pnpmPath = $pnpmTool.Source }
else {
    $pnpmPath = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd'
}
if (!(Test-Path -LiteralPath $pnpmPath)) { throw 'Install Node.js and pnpm, then rerun scripts/build-desktop.ps1.' }
$nodeTool = Get-Command node.exe -ErrorAction SilentlyContinue
$uiOriginalPath = $env:PATH
try {
    if (!$nodeTool) {
        $nodeFolder = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin'
        if (!(Test-Path -LiteralPath "$nodeFolder\node.exe")) { throw 'Node.js is missing.' }
        $env:PATH = "$nodeFolder;" + $env:PATH
    }
    # Directory moves can require regenerating node_modules; builds must also work without a TTY.
    $installArguments = @('--dir','airplay-frontend','install','--frozen-lockfile','--config.confirmModulesPurge=false')
    if ($Offline) { $installArguments += '--offline' }
    & $pnpmPath @installArguments
    if ($LASTEXITCODE) { throw 'UI dependency installation failed' }
    & $pnpmPath --dir airplay-frontend build
    if ($LASTEXITCODE) { throw 'UI frontend build failed' }
    $arguments = @('build','--manifest-path','airplay-frontend/src-tauri/Cargo.toml','--locked')
    if ($Offline) { $arguments += '--offline' }
    if ($Release) { $arguments += '--release' }
    & $cargo @arguments
    if ($LASTEXITCODE) { throw 'UI Rust build failed' }
    $profileFolder = if ($Release) { 'release' } else { 'debug' }
    New-Item -ItemType Directory -Force -Path dist,dist/docs | Out-Null
    Copy-Item -LiteralPath LICENSE,NOTICE -Destination dist
    Copy-Item -LiteralPath scripts/distribution-readme.md -Destination dist/docs/README.md
    Copy-Item -LiteralPath CHANGELOG.md -Destination dist/docs/release-notes.md
    Copy-Item -LiteralPath docs/licensing.md -Destination dist/docs/licensing.md
    Copy-Item -LiteralPath docs/error-codes.md -Destination dist/docs/error-codes.md
    Copy-Item -LiteralPath "airplay-frontend/src-tauri/target/$profileFolder/airplay-bridge.exe" -Destination dist/airplay-bridge.exe
    # Retire the previous backend name only after the GUI has also been updated.
    $legacyBackend = Join-Path $projectRoot 'dist/runtime/cliairplay-probe.exe'
    if (Test-Path -LiteralPath $legacyBackend) { Remove-Item -LiteralPath $legacyBackend }
    Write-Host 'Built: dist/airplay-bridge.exe. Keep the complete dist folder together.'
}
finally { $env:PATH = $uiOriginalPath }
