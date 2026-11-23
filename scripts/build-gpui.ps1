param([switch]$Offline, [switch]$Production, [switch]$DebugBuild)

$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    $runtime = 'dist/runtime'
    if (!(Test-Path -LiteralPath "$runtime/airplay-backend.exe" -PathType Leaf)) {
        throw 'Run scripts/build-backend.ps1 first to populate dist/runtime.'
    }

    $target = 'x86_64-pc-windows-msvc'
    $targetDir = 'build/gpui-migration-target'
    $arguments = @('build', '--manifest-path', 'airplay-frontend/src-gpui/Cargo.toml', '--target', $target, '--target-dir', $targetDir, '--locked')
    $arguments += @('--config', 'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]')
    $profileFolder = if ($DebugBuild) { 'debug' } else { 'release' }
    if (!$DebugBuild) { $arguments += '--release' }
    if ($Offline) { $arguments += '--offline' }
    else { $arguments += @('--config', 'net.offline=false') }
    & cargo @arguments
    if ($LASTEXITCODE) { throw 'GPUI build failed.' }
    $gui = "$targetDir/$target/$profileFolder/airplay-bridge-gpui.exe"
    if ($Production) {
        Copy-Item -LiteralPath $gui -Destination 'dist/airplay-bridge.exe'
        Write-Host 'Built: dist/airplay-bridge.exe'
        return
    }

    # 预览版在可执行文件旁使用独立的便携数据目录。
    # The preview uses its own portable data directory alongside the executable.
    $preview = 'build/gpui-preview'
    New-Item -ItemType Directory -Force -Path "$preview/runtime" | Out-Null
    Copy-Item -LiteralPath $gui -Destination $preview
    Copy-Item -LiteralPath "$runtime/airplay-backend.exe" -Destination "$preview/runtime"
    Get-ChildItem -LiteralPath $runtime -Filter '*.dll' -File | Copy-Item -Destination "$preview/runtime"
    New-Item -ItemType File -Force -Path "$preview/portable.flag" | Out-Null
    Write-Host "Built: $preview/airplay-bridge-gpui.exe"
}
finally { Pop-Location }
