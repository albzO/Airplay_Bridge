param([switch]$Offline, [switch]$Release, [switch]$SkipBackend)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
if (!$SkipBackend) {
    & "$PSScriptRoot\Build.ps1" -Offline:$Offline
}
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$pnpmTool = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
if ($pnpmTool) { $pnpmPath = $pnpmTool.Source }
else {
    $pnpmPath = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd'
}
if (!(Test-Path -LiteralPath $pnpmPath)) { throw 'Install Node.js and pnpm, then rerun Build-UI.ps1.' }
$nodeTool = Get-Command node.exe -ErrorAction SilentlyContinue
$uiOriginalPath = $env:PATH
try {
    if (!$nodeTool) {
        $nodeFolder = Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin'
        if (!(Test-Path -LiteralPath "$nodeFolder\node.exe")) { throw 'Node.js is missing.' }
        $env:PATH = "$nodeFolder;" + $env:PATH
    }
    $installArguments = @('--dir','desktop','install','--frozen-lockfile')
    if ($Offline) { $installArguments += '--offline' }
    & $pnpmPath @installArguments
    if ($LASTEXITCODE) { throw 'UI dependency installation failed' }
    & $pnpmPath --dir desktop build
    if ($LASTEXITCODE) { throw 'UI frontend build failed' }
    $arguments = @('build','--manifest-path','desktop/src-tauri/Cargo.toml','--locked')
    if ($Offline) { $arguments += '--offline' }
    if ($Release) { $arguments += '--release' }
    & $cargo @arguments
    if ($LASTEXITCODE) { throw 'UI Rust build failed' }
    $profileFolder = if ($Release) { 'release' } else { 'debug' }
    Copy-Item -LiteralPath "desktop/src-tauri/target/$profileFolder/airplay-bridge.exe" -Destination dist/airplay-bridge.exe
    Write-Host 'Built: dist/airplay-bridge.exe. Keep the complete dist folder together.'
}
finally { $env:PATH = $uiOriginalPath }
