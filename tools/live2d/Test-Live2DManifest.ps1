[CmdletBinding()] param([Parameter(Mandatory)][string]$ManifestPath,[Parameter(Mandatory)][string]$SourceRoot,[string[]]$AssetId)
$ErrorActionPreference='Stop'
try {
    Import-Module (Join-Path $PSScriptRoot 'Live2DManifest.psm1') -Force
    $parsed=Read-Live2DManifest $ManifestPath
    $ids=@(); foreach($value in @($AssetId)){ $ids+=@($value -split ','|Where-Object{$_}) }
    if(-not $ids.Count){$ids=@($parsed.Document.assets.id)}
    $assets=Test-Live2DManifestAssets $parsed.Document $SourceRoot $ids
    [ordered]@{schemaVersion=1;assets=@($assets|ForEach-Object{[ordered]@{id=$_.id;fileCount=$_.files.Count}})}|ConvertTo-Json -Depth 5
    exit 0
} catch {[Console]::Error.WriteLine($_.Exception.Message);exit 1}
