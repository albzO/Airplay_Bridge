param([switch]$Offline, [switch]$Release, [switch]$SkipBackend)

$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    if (!$SkipBackend) {
        & "$PSScriptRoot/build-backend.ps1" -Offline:$Offline
    }
    & "$PSScriptRoot/build-gpui.ps1" -Production -Offline:$Offline -DebugBuild:(!$Release)

    if ($Release) {
        $arguments = @('build', '--manifest-path', 'airplay-core/Cargo.toml', '--release', '--locked')
        $arguments += @('--config', 'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]')
        if ($Offline) { $arguments += '--offline' }
        else { $arguments += @('--config', 'net.offline=false') }
        & cargo @arguments
        if ($LASTEXITCODE) { throw 'Release CLI build failed.' }
        New-Item -ItemType Directory -Force -Path dist/tools | Out-Null
        Copy-Item -LiteralPath airplay-core/target/release/homepod-test.exe -Destination dist/tools
    }

    New-Item -ItemType Directory -Force -Path dist/docs | Out-Null
    Copy-Item -LiteralPath LICENSE,NOTICE -Destination dist
    Copy-Item -LiteralPath THIRD_PARTY.md -Destination dist/docs
    Copy-Item -LiteralPath scripts/distribution-readme.md -Destination dist/docs/README.md
    Copy-Item -LiteralPath CHANGELOG.md -Destination dist/docs/release-notes.md
    Copy-Item -LiteralPath docs/playback-loopback.md,docs/licensing.md,docs/error-codes.md -Destination dist/docs
    Write-Host 'Keep the complete dist folder together.'
}
finally { Pop-Location }
