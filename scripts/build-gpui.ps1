# 构建桌面界面（src-ely）。
# Builds the desktop UI (src-ely).
param([switch]$Offline, [switch]$Production, [switch]$DebugBuild)

$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
# Ely 把图标和字体许可证放在子目录里，collect-rust-notices.py 只收集包根目录，这里单独复制。
# Ely keeps its icon and font licenses in subfolders that collect-rust-notices.py does not scan.
function Copy-ElyLicenses([bool]$Offline) {
    $metadataArguments = @('metadata', '--manifest-path', 'airplay-frontend/src-ely/Cargo.toml', '--format-version', '1', '--locked')
    if ($Offline) { $metadataArguments += '--offline' }
    $metadata = ((& cargo @metadataArguments) -join "`n") | ConvertFrom-Json
    if ($LASTEXITCODE) { throw 'Could not read the src-ely dependency graph.' }
    $package = $metadata.packages | Where-Object name -eq 'ely-core' | Select-Object -First 1
    $ely = Split-Path -Parent $package.manifest_path
    $destination = 'dist/licenses/ely'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    foreach ($pair in @(@('LICENSE', 'ely-LICENSE'), @('icons/LICENSE', 'lucide-icons-LICENSE'),
            @('assets/fonts/Inter-LICENSE.txt', 'Inter-LICENSE.txt'),
            @('assets/fonts/JetBrainsMono-LICENSE.txt', 'JetBrainsMono-LICENSE.txt'),
            @('assets/plex/IBMPlexSans-LICENSE.txt', 'IBMPlexSans-LICENSE.txt'))) {
        Copy-Item -LiteralPath (Join-Path $ely $pair[0]) -Destination (Join-Path $destination $pair[1])
    }
}

try {
    $runtime = 'dist/runtime'
    if (!(Test-Path -LiteralPath "$runtime/airplay-backend.exe" -PathType Leaf)) {
        throw 'Run scripts/build-backend.ps1 first to populate dist/runtime.'
    }

    $target = 'x86_64-pc-windows-msvc'
    $targetDir = 'build/ely-target'
    $arguments = @('build', '--manifest-path', 'airplay-frontend/src-ely/Cargo.toml', '--target', $target, '--target-dir', $targetDir, '--locked')
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
        Copy-ElyLicenses $Offline
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
