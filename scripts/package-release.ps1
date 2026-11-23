param(
    [switch]$Offline,
    [switch]$SkipBuild,
    [switch]$IncludeNsis,
    [string]$Iscc = '',
    [string]$Makensis = '',
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    if (!$SkipBuild) { & "$PSScriptRoot/build-desktop.ps1" -Release -Offline:$Offline }
    $metadataArguments = @('metadata', '--manifest-path', 'airplay-frontend/src-gpui/Cargo.toml', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc', '--locked')
    if ($Offline) { $metadataArguments += '--offline' }
    else { $metadataArguments += @('--config', 'net.offline=false') }
    $metadataJson = (& cargo @metadataArguments) -join "`n"
    if ($LASTEXITCODE) { throw 'Could not read the Windows dependency graph.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object id -eq $metadata.resolve.root).version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected a numeric release version.' }
    if ((Get-Item -LiteralPath 'dist/airplay-bridge.exe').VersionInfo.FileVersion -ne $version) {
        throw "dist/airplay-bridge.exe does not match Cargo version $version. Rebuild before packaging."
    }

    if (!$Iscc) {
        $tool = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($tool) { $Iscc = $tool.Source }
        else { $Iscc = Join-Path $env:ProgramFiles 'Inno Setup 7/ISCC.exe' }
    }
    if (!(Test-Path -LiteralPath $Iscc -PathType Leaf)) { throw 'Provide the Inno Setup compiler with -Iscc.' }
    if ($IncludeNsis -and !$Makensis) {
        $tool = Get-Command makensis.exe -ErrorAction SilentlyContinue
        if ($tool) { $Makensis = $tool.Source }
        else { $Makensis = Join-Path $projectRoot '.local/nsis-3.11/makensis.exe' }
    }
    if ($IncludeNsis -and !(Test-Path -LiteralPath $Makensis -PathType Leaf)) { throw 'Provide the NSIS compiler with -Makensis.' }
    $output = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory, $projectRoot) }
        else { Join-Path $projectRoot "releases/v$version" }
    New-Item -ItemType Directory -Force -Path $output | Out-Null

    # 新建暂存目录，避免将本地运行数据和过时文件带入发布包。
    # A fresh staging directory keeps local runtime data and stale files out of release packages.
    $stage = Join-Path $projectRoot ('.local/package-' + [guid]::NewGuid().ToString('N'))
    $payload = Join-Path $stage "AirPlay-Bridge-$version"
    New-Item -ItemType Directory -Force -Path "$payload/runtime", "$payload/tools", "$payload/docs", "$payload/licenses" | Out-Null
    Copy-Item -LiteralPath 'dist/airplay-bridge.exe', 'dist/LICENSE', 'dist/NOTICE' -Destination $payload
    Copy-Item -LiteralPath 'dist/runtime/airplay-backend.exe' -Destination "$payload/runtime"
    Get-ChildItem -LiteralPath 'dist/runtime' -Filter '*.dll' -File | Copy-Item -Destination "$payload/runtime"
    Copy-Item -LiteralPath 'dist/tools/homepod-test.exe' -Destination "$payload/tools"
    foreach ($file in @('README.md', 'release-notes.md', 'playback-loopback.md', 'licensing.md', 'error-codes.md', 'THIRD_PARTY.md')) {
        Copy-Item -LiteralPath "dist/docs/$file" -Destination "$payload/docs"
    }
    foreach ($file in @('airplay-cli-LICENSE', 'crosstools-LICENSE', 'openssl/LICENSE', 'winpthreads/COPYING', 'libwinpthread/COPYING')) {
        $source = Join-Path $projectRoot "dist/licenses/$file"
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
            if ($file -in @('airplay-cli-LICENSE', 'crosstools-LICENSE', 'openssl/LICENSE')) {
                throw "Missing distribution license: $file"
            }
            continue
        }
        $destination = Join-Path $payload "licenses/$file"
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination
    }
    $metadataPath = Join-Path $stage 'cargo-metadata.json'
    [IO.File]::WriteAllText($metadataPath, $metadataJson)
    & python "$PSScriptRoot/collect-rust-notices.py" $metadataPath "$payload/licenses/rust"
    if ($LASTEXITCODE) { throw 'Could not collect Rust license notices.' }

    $installer = Join-Path $output "AirPlay-Bridge-$version-windows-x64-setup.exe"
    & $Iscc /Q "/O$output" "/DMyAppVersion=$version" "/DPayloadDir=$payload" "$PSScriptRoot/installpackcompiler.iss"
    if ($LASTEXITCODE) { throw 'Inno Setup compilation failed.' }
    $artifacts = @($installer)
    if ($IncludeNsis) {
        # 只卸载发行清单中的文件和清空后的目录。
        # Remove only listed release files and the resulting empty directories.
        $deleteLines = foreach ($file in Get-ChildItem -LiteralPath $payload -Recurse -File) {
            $relative = $file.FullName.Substring($payload.Length + 1).Replace('$', '$$')
            'Delete "$INSTDIR\' + $relative + '"'
        }
        $deleteLines += foreach ($directory in Get-ChildItem -LiteralPath $payload -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending) {
            $relative = $directory.FullName.Substring($payload.Length + 1).Replace('$', '$$')
            'RMDir "$INSTDIR\' + $relative + '"'
        }
        $manifest = Join-Path $stage 'uninstall-files.nsh'
        [IO.File]::WriteAllLines($manifest, [string[]]$deleteLines, [Text.UTF8Encoding]::new($false))
        $nsis = Join-Path $output "AirPlay-Bridge-$version-windows-x64-nsis-setup.exe"
        & $Makensis /V2 "/DVERSION=$version" "/DPAYLOAD=$payload" "/DOUTPUT=$nsis" "/DUNINSTALL_FILES=$manifest" "$PSScriptRoot/installer.nsi"
        if ($LASTEXITCODE) { throw 'NSIS compilation failed.' }
        $artifacts += $nsis
    }

    # 便携标记在安装包编译完成后添加。
    # Add the portable marker only after compiling the installers.
    [IO.File]::WriteAllText((Join-Path $payload 'portable.flag'), "Portable mode: store application data in data/.`n")
    $archive = Join-Path $output "AirPlay-Bridge-$version-windows-x64-portable.zip"
    Compress-Archive -LiteralPath $payload -DestinationPath $archive -Force
    $artifacts += $archive
    $hashes = foreach ($file in $artifacts) {
        $hash = Get-FileHash -LiteralPath $file -Algorithm SHA256
        $hash.Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($file)
    }
    [IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS.txt'), [string[]]$hashes)
    foreach ($file in $artifacts) { Write-Host "Built: $file" }
}
finally { Pop-Location }
