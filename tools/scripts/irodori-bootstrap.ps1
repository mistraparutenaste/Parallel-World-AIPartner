[CmdletBinding()]
param(
    [string] $ManifestPath,
    [string] $DataRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($ManifestPath)) {
    $repositoryRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..')
    $ManifestPath = Join-Path $repositoryRoot 'content\runtime-manifests\irodori\windows-x86_64.json'
}
if ([string]::IsNullOrWhiteSpace($DataRoot)) {
    $DataRoot = Join-Path $env:LOCALAPPDATA 'com.parallelworld.desktop\irodori'
}

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
