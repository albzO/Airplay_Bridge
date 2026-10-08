param(
    [switch]$Offline,
    [switch]$SkipBuild,
    [string]$Makensis = '',
    [string]$WebViewBootstrapper = ''
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $projectRoot
if (!$SkipBuild) { & "$PSScriptRoot/build-desktop.ps1" -Release -Offline:$Offline }
$config = Get-Content 'airplay-frontend/src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json
$version = $config.version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected a numeric release version.' }
if (!$Makensis) {
    $tool = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($tool) { $Makensis = $tool.Source }
    else { $Makensis = Join-Path $projectRoot '.local/nsis-3.11/makensis.exe' }
}
if (!$WebViewBootstrapper) { $WebViewBootstrapper = Join-Path $projectRoot '.local/MicrosoftEdgeWebview2Setup.exe' }
foreach ($tool in @($Makensis, $WebViewBootstrapper)) {
    if (!(Test-Path -LiteralPath $tool -PathType Leaf)) { throw "Missing packaging tool: $tool. See docs/building.md." }
}
$signature = Get-AuthenticodeSignature -LiteralPath $WebViewBootstrapper
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
    throw 'WebView2 bootstrapper must have a valid Microsoft signature.'
}
$output = Join-Path $projectRoot 'releases'
New-Item -ItemType Directory -Force -Path $output | Out-Null
# A fresh staging directory prevents local runtime data and obsolete files entering a release.
$stage = Join-Path $projectRoot ('.local/package-' + [guid]::NewGuid().ToString('N'))
$payload = Join-Path $stage "AirPlay-Bridge-$version"
New-Item -ItemType Directory -Force -Path $payload | Out-Null
foreach ($file in @('airplay-bridge.exe', 'LICENSE', 'NOTICE')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot "dist/$file") -Destination $payload
}
New-Item -ItemType Directory -Force -Path "$payload/runtime", "$payload/tools", "$payload/docs", "$payload/licenses" | Out-Null
Copy-Item -LiteralPath 'dist/runtime/airplay-backend.exe' -Destination "$payload/runtime"
Get-ChildItem -LiteralPath 'dist/runtime' -Filter '*.dll' -File | Copy-Item -Destination "$payload/runtime"
Copy-Item -LiteralPath 'dist/tools/homepod-test.exe' -Destination "$payload/tools"
foreach ($file in @('README.md', 'release-notes.md', 'licensing.md', 'error-codes.md', 'THIRD_PARTY.md')) {
    Copy-Item -LiteralPath "dist/docs/$file" -Destination "$payload/docs"
}
Get-ChildItem -LiteralPath 'dist/licenses' | Copy-Item -Destination "$payload/licenses" -Recurse
# Generate explicit uninstall paths: never recursively delete a user-selected directory.
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
$installer = Join-Path $output "AirPlay-Bridge-$version-windows-x64-setup.exe"
& $Makensis /V2 "/DVERSION=$version" "/DPAYLOAD=$payload" "/DOUTPUT=$installer" "/DUNINSTALL_FILES=$manifest" "/DWEBVIEW_BOOTSTRAPPER=$WebViewBootstrapper" "$PSScriptRoot/installer.nsi"
if ($LASTEXITCODE) { throw 'Installer compilation failed.' }
[IO.File]::WriteAllText((Join-Path $payload 'portable.flag'), "Portable mode: store application data in data/.`n")
$archive = Join-Path $output "AirPlay-Bridge-$version-windows-x64-portable.zip"
Compress-Archive -LiteralPath $payload -DestinationPath $archive -Force
$hashes = foreach ($file in @($archive, $installer)) {
    $hash = Get-FileHash -LiteralPath $file -Algorithm SHA256
    $hash.Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($file)
}
[IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS.txt'), [string[]]$hashes)
Write-Host "Portable: $archive"
Write-Host "Installer: $installer"
