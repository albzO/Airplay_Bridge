# Compatibility entry point. New usage: scripts/build-desktop.ps1.
param([switch]$Offline, [switch]$Release, [switch]$SkipBackend)
& "$PSScriptRoot\scripts\build-desktop.ps1" @PSBoundParameters
