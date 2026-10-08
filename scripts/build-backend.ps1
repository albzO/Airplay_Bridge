param(
    [string]$MsysRoot = 'C:\msys64',
    [string]$Python = '',
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $projectRoot
$bin = Join-Path $MsysRoot 'ucrt64\bin'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (!$Python) {
    $installed = Get-Command python.exe -ErrorAction SilentlyContinue
    if ($installed) { $Python = $installed.Source }
    else { $Python = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\python.exe' }
}
foreach ($tool in @("$bin\cmake.exe", "$bin\ninja.exe", "$bin\gcc.exe", "$bin\g++.exe", "$bin\objdump.exe", $cargo, $Python)) {
    if (!(Test-Path -LiteralPath $tool)) { throw "Missing development tool: $tool" }
}
if (!(Test-Path 'upstream/airplay-cli/libraop/crosstools/src/cross_log.h')) {
    throw 'Missing pinned upstream sources. See README.md; do not update them implicitly.'
}
$oldPath = $env:PATH
try {
    $env:PATH = "$bin;" + $env:PATH
    & "$bin\cmake.exe" -S airplay-backend -B build/airplay-backend -G Ninja `
        "-DCMAKE_C_COMPILER=$bin/gcc.exe" "-DCMAKE_CXX_COMPILER=$bin/g++.exe" `
        "-DCMAKE_MAKE_PROGRAM=$bin/ninja.exe" "-DPython3_EXECUTABLE=$Python" `
        "-DOPENSSL_ROOT_DIR=$MsysRoot/ucrt64" -DCMAKE_BUILD_TYPE=Debug
    if ($LASTEXITCODE) { throw 'CMake configure failed' }
    & "$bin\cmake.exe" --build build/airplay-backend --parallel 4
    if ($LASTEXITCODE) { throw 'C/C++ build failed' }
    $cargoArgs = @('build', '--manifest-path', 'airplay-core/Cargo.toml', '--locked')
    if ($Offline) { $cargoArgs += '--offline' }
    else { $cargoArgs += @('--config', 'net.offline=false') }
    & $cargo @cargoArgs
    if ($LASTEXITCODE) { throw 'Rust build failed' }
    New-Item -ItemType Directory -Force -Path dist/runtime,dist/tools,dist/docs | Out-Null
    Copy-Item -LiteralPath build/airplay-backend/airplay-backend.exe -Destination dist/runtime
    Copy-Item -LiteralPath airplay-core/target/debug/homepod-test.exe -Destination dist/tools
    # Package only DLL imports found in this UCRT64 toolchain, recursively.
    $queue = [System.Collections.Generic.Queue[string]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    $queue.Enqueue((Join-Path $projectRoot 'dist/runtime/airplay-backend.exe'))
    while ($queue.Count) {
        $binary = $queue.Dequeue()
        $imports = & "$bin\objdump.exe" -p $binary
        if ($LASTEXITCODE) { throw "Cannot inspect DLL imports: $binary" }
        foreach ($line in $imports) {
            if ($line -match 'DLL Name:\s*(\S+)') {
                $dll = $Matches[1]
                $source = Join-Path $bin $dll
                if ((Test-Path -LiteralPath $source) -and $seen.Add($dll)) {
                    Copy-Item -LiteralPath $source -Destination dist/runtime
                    $queue.Enqueue((Join-Path $projectRoot "dist/runtime/$dll"))
                }
            }
        }
    }
    New-Item -ItemType Directory -Force -Path dist/licenses | Out-Null
    Copy-Item -LiteralPath upstream/airplay-cli/LICENSE -Destination dist/licenses/airplay-cli-LICENSE
    Copy-Item -LiteralPath upstream/airplay-cli/libraop/crosstools/LICENSE -Destination dist/licenses/crosstools-LICENSE
    foreach ($library in @('openssl', 'winpthreads', 'libwinpthread')) {
        $directory = Join-Path $MsysRoot "ucrt64/share/licenses/$library"
        if (Test-Path -LiteralPath $directory) { Copy-Item -LiteralPath $directory -Destination dist/licenses -Recurse -Force }
    }
    Copy-Item -LiteralPath THIRD_PARTY.md -Destination dist/docs
    Copy-Item -LiteralPath LICENSE,NOTICE -Destination dist
    Copy-Item -LiteralPath scripts/distribution-readme.md -Destination dist/docs/README.md
    Copy-Item -LiteralPath CHANGELOG.md -Destination dist/docs/release-notes.md
    Copy-Item -LiteralPath docs/playback-loopback.md -Destination dist/docs/playback-loopback.md
    Copy-Item -LiteralPath docs/licensing.md -Destination dist/docs/licensing.md
    Copy-Item -LiteralPath docs/error-codes.md -Destination dist/docs/error-codes.md
    & .\dist\runtime\airplay-backend.exe --self-check
    if ($LASTEXITCODE) { throw 'Packaged backend self-check failed' }
    Write-Host 'Built: dist/tools/homepod-test.exe and dist/runtime/airplay-backend.exe'
} finally { $env:PATH = $oldPath }
