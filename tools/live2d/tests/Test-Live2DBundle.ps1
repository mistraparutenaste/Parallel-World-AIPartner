$ErrorActionPreference='Stop';Set-StrictMode -Version Latest
$repoRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$dist=Join-Path $repoRoot 'apps/desktop/dist'
if(-not(Test-Path -LiteralPath $dist)){throw 'Desktop dist is missing; run the build first.'}
$forbiddenDirectories=@((Join-Path $dist 'static/live2d-dev'),(Join-Path $dist '.dev-assets'))
foreach($path in $forbiddenDirectories){if(Test-Path -LiteralPath $path){throw "Unapproved Live2D directory entered dist: $path"}}
$stagedReceipt=Join-Path $repoRoot '.dev-assets/live2d/staging-manifest.json'
$hashes=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
if(Test-Path -LiteralPath $stagedReceipt){foreach($entry in (Get-Content -Raw $stagedReceipt|ConvertFrom-Json).files){[void]$hashes.Add($entry.sha256)}}
foreach($file in @(Get-ChildItem -Recurse -File -LiteralPath $dist)){
    if($file.Name -match '^(Mark|Epsilon|live2dcubismcore)' -or $file.Name -eq 'staging-manifest.json'){throw "Unapproved Live2D filename entered dist: $($file.FullName)"}
    if($hashes.Count -and $hashes.Contains((Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash)){throw "Staged Live2D bytes entered dist: $($file.FullName)"}
}
$forbiddenText='__live2d_dev__|ParallelWorldCubismR5Bridge|readPixels|live2dAlphaPixels'
$textFiles=@(Get-ChildItem -Recurse -File -LiteralPath $dist|Where-Object {$_.Extension -in '.js','.html','.css','.json','.map'})
$matches=@($textFiles|Select-String -Pattern $forbiddenText)
if($matches.Count){throw "Development-only Live2D identifier entered production dist: $($matches[0].Path)"}
Write-Host 'DIST_LIVE2D_DEV_IDENTIFIERS=0' -ForegroundColor Green
Write-Host 'DIST_UNAPPROVED_LIVE2D_FILES=0' -ForegroundColor Green
