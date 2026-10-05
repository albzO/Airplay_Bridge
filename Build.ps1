# Compatibility entry point. New usage: scripts/build-backend.ps1.
param([string]$MsysRoot = 'C:\msys64', [string]$Python = '', [switch]$Offline)
& "$PSScriptRoot\scripts\build-backend.ps1" @PSBoundParameters
