Set-StrictMode -Version Latest

function Throw-ManifestError([string]$Message) { throw "Live2D manifest validation failed: $Message" }
function Assert-KnownProperties($Object, [string[]]$Allowed, [string]$Context) {
    if ($null -eq $Object -or $Object -isnot [pscustomobject]) { Throw-ManifestError "$Context must be an object." }
    foreach ($property in $Object.PSObject.Properties.Name) {
        if ($Allowed -notcontains $property) { Throw-ManifestError "$Context has unknown property '$property'." }
    }
}
function Get-Live2DSafeSegments([string]$Value, [string]$Context) {
    if ([string]::IsNullOrWhiteSpace($Value) -or [IO.Path]::IsPathRooted($Value) -or $Value -match '^[A-Za-z]:') { Throw-ManifestError "$Context must be a nonempty relative path." }
    $segments = @($Value -split '[\\/]' | Where-Object { $_ })
    if ($segments.Count -eq 0 -or @($segments | Where-Object { $_ -eq '.' -or $_ -eq '..' }).Count) { Throw-ManifestError "$Context escapes its root." }
    $segments
}
function Resolve-Live2DExactPath([string]$Root, [string]$Relative, [bool]$File) {
    $current = Get-Item -LiteralPath $Root -Force
    foreach ($segment in (Get-Live2DSafeSegments $Relative 'path')) {
        if (($current.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { Throw-ManifestError "reparse point is forbidden: $($current.FullName)" }
        $matches = @(Get-ChildItem -Force -LiteralPath $current.FullName | Where-Object { [string]::Equals($_.Name,$segment,[StringComparison]::Ordinal) })
        if ($matches.Count -ne 1) { Throw-ManifestError "missing path or case mismatch: $Relative" }
        $current = $matches[0]
    }
    if (($current.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or ($File -and $current.PSIsContainer)) { Throw-ManifestError "invalid leaf: $Relative" }
    $current
}
function Assert-Live2DNoReparseAncestors([string]$Path) {
    $cursor=[IO.Path]::GetFullPath($Path)
    while($cursor){if(Test-Path -LiteralPath $cursor){$item=Get-Item -Force -LiteralPath $cursor;if(($item.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne 0){Throw-ManifestError "reparse ancestor forbidden: $cursor"}};$parent=Split-Path -Parent $cursor;if($parent-eq$cursor){break};$cursor=$parent}
}
function Assert-Live2DNoNestedReparse([string]$Root) {
    $pending=[Collections.Generic.Queue[string]]::new();$pending.Enqueue((Get-Item -LiteralPath $Root).FullName)
    while($pending.Count){$directory=$pending.Dequeue();foreach($entry in @(Get-ChildItem -Force -LiteralPath $directory)){if(($entry.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne 0){Throw-ManifestError "nested reparse point forbidden: $($entry.FullName)"};if($entry.PSIsContainer){$pending.Enqueue($entry.FullName)}}}
}
function Read-Live2DManifest([string]$ManifestPath) {
    Assert-Live2DNoReparseAncestors $ManifestPath
    $bytes=[IO.File]::ReadAllBytes($ManifestPath)
    $sha=[Security.Cryptography.SHA256]::Create();try{$rawHash=([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-','').ToLowerInvariant()}finally{$sha.Dispose()}
    try { $raw=[Text.UTF8Encoding]::new($false,$true).GetString($bytes);$manifest = $raw | ConvertFrom-Json } catch { Throw-ManifestError 'manifest must be valid UTF-8 JSON.' }
    Assert-KnownProperties $manifest @('schemaVersion','generatedAt','assets') 'root'
    if ($manifest.schemaVersion -isnot [int] -or $manifest.schemaVersion -ne 1) { Throw-ManifestError 'schemaVersion must be integer 1.' }
    if($manifest.generatedAt -isnot [string] -or [string]::IsNullOrWhiteSpace($manifest.generatedAt)){Throw-ManifestError 'generatedAt must be a nonempty string.'}
    if ($manifest.assets -isnot [array] -or $manifest.assets.Count -eq 0) { Throw-ManifestError 'assets must be a nonempty array.' }
    $ids = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($asset in $manifest.assets) {
        Assert-KnownProperties $asset @('id','name','sourceType','sourceUrl','sourceVersion','sourceCommit','localPath','licenseCategory','intendedUse','redistributionApproved','files') "asset"
        if ($asset.id -isnot [string] -or [string]::IsNullOrWhiteSpace($asset.id) -or -not $ids.Add($asset.id)) { Throw-ManifestError 'asset id must be nonempty and unique.' }
        foreach($field in @('name','sourceType','sourceUrl','sourceVersion','licenseCategory')){if($asset.$field -isnot [string] -or [string]::IsNullOrWhiteSpace($asset.$field)){Throw-ManifestError "asset '$($asset.id)' $field must be a nonempty string."}}
        if($asset.PSObject.Properties.Name -contains 'sourceCommit'){if($null-ne$asset.sourceCommit -and $asset.sourceCommit -isnot [string]){Throw-ManifestError "asset '$($asset.id)' sourceCommit must be null or a string when present."}}
        if($asset.intendedUse -isnot [array] -or $asset.intendedUse.Count-eq 0 -or @($asset.intendedUse|Where-Object{$_ -isnot [string] -or [string]::IsNullOrWhiteSpace($_)}).Count){Throw-ManifestError "asset '$($asset.id)' intendedUse must be a nonempty string array."}
        if($asset.redistributionApproved -isnot [bool]){Throw-ManifestError "asset '$($asset.id)' redistributionApproved must be boolean."}
        if ($asset.localPath -isnot [string]) { Throw-ManifestError "asset '$($asset.id)' localPath must be a string." }
        [void](Get-Live2DSafeSegments $asset.localPath "asset '$($asset.id)' localPath")
        if ($asset.files -isnot [array] -or $asset.files.Count -eq 0) { Throw-ManifestError "asset '$($asset.id)' files must be a nonempty array." }
        $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($file in $asset.files) {
            Assert-KnownProperties $file @('path','size','sha256') "asset '$($asset.id)' file"
            if ($file.path -isnot [string]) { Throw-ManifestError 'file path must be a string.' }
            $canonical = (Get-Live2DSafeSegments $file.path 'file path') -join '/'
            if (-not $paths.Add($canonical)) { Throw-ManifestError "duplicate file path: $canonical" }
            if (($file.size -isnot [int]) -and ($file.size -isnot [long])) { Throw-ManifestError "file size must be an integer: $canonical" }
            if ([int64]$file.size -lt 0) { Throw-ManifestError "file size must be nonnegative: $canonical" }
            if ($file.sha256 -isnot [string] -or $file.sha256 -cnotmatch '^[0-9a-fA-F]{64}$') { Throw-ManifestError "sha256 must be exactly 64 hex characters: $canonical" }
        }
    }
    [pscustomobject]@{ Document=$manifest; RawHash=$rawHash }
}
function Test-Live2DManifestAssets($Manifest, [string]$SourceRoot, [string[]]$AssetIds) {
    $root = Get-Item -LiteralPath $SourceRoot -Force
    if (-not $root.PSIsContainer -or ($root.Attributes -band [IO.FileAttributes]::ReparsePoint)) { Throw-ManifestError 'SourceRoot must be a real directory.' }
    Assert-Live2DNoReparseAncestors $root.FullName
    $assets = @($Manifest.assets | Where-Object { $AssetIds -contains $_.id })
    foreach ($id in $AssetIds) { if (@($assets | Where-Object id -eq $id).Count -ne 1) { Throw-ManifestError "missing asset: $id" } }
    foreach ($asset in $assets) {
        $assetRoot = Resolve-Live2DExactPath $root.FullName $asset.localPath $false
        if (-not $assetRoot.PSIsContainer) { Throw-ManifestError "asset root is not a directory: $($asset.id)" }
        Assert-Live2DNoNestedReparse $assetRoot.FullName
        $declared = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($file in $asset.files) {
            $relative = (Get-Live2DSafeSegments $file.path 'file path') -join '/'; [void]$declared.Add($relative)
            $item = Resolve-Live2DExactPath $assetRoot.FullName $relative $true
            if ($item.Length -ne [int64]$file.size -or (Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName).Hash.ToLowerInvariant() -ne $file.sha256.ToLowerInvariant()) { Throw-ManifestError "size/hash mismatch: $($asset.id)/$relative" }
        }
        $actual = @(Get-ChildItem -Recurse -File -Force -LiteralPath $assetRoot.FullName | ForEach-Object { $_.FullName.Substring($assetRoot.FullName.Length).TrimStart('\','/').Replace('\','/') })
        foreach ($path in $actual) { if (-not $declared.Contains($path)) { Throw-ManifestError "undeclared file: $($asset.id)/$path" } }
        if ($actual.Count -ne $declared.Count) { Throw-ManifestError "declared/source set mismatch: $($asset.id)" }
    }
    $assets
}
Export-ModuleMember -Function Read-Live2DManifest,Test-Live2DManifestAssets,Get-Live2DSafeSegments,Assert-Live2DNoReparseAncestors,Assert-Live2DNoNestedReparse
