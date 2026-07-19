[CmdletBinding()]
param(
    [string] $ManifestPath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..\..')) 'content\runtime-manifests\irodori\windows-x86_64.json'),
    [string] $DataRoot = (Join-Path $env:LOCALAPPDATA 'com.parallelworld.desktop\irodori')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'irodori-bootstrap.psm1') -Force

try {
    $result = Invoke-IrodoriBootstrap -ManifestPath $ManifestPath -DataRoot $DataRoot
    exit ([int] $result.app_exit_code)
} catch [OperationCanceledException] {
    Write-Host 'Irodori setup was cancelled.' -ForegroundColor Yellow
    exit 130
} catch [Management.Automation.PipelineStoppedException] {
    exit 130
} catch {
    Write-Host 'Parallel World startup failed.' -ForegroundColor Yellow
    exit 1
}
