[CmdletBinding()] param(
    [Parameter(Mandatory)][string]$SourceRoot,
    [string]$ManifestPath,
    [string]$DestinationRoot,
    [string[]]$Model=@('live2d-mark'),
    [string]$FrameworkRelativePath='third_party/live2d/CubismWebSamples/Framework/src'
)
$ErrorActionPreference='Stop'; Set-StrictMode -Version Latest
$repoRoot=[IO.Path]::GetFullPath((Split-Path -Parent (Split-Path -Parent $PSScriptRoot))).TrimEnd('\')
$defaultDestination=Join-Path $repoRoot '.dev-assets/live2d'
if(-not $ManifestPath){$ManifestPath=Join-Path $repoRoot 'project-input/live2d/manifests/assets.json'}
if(-not $DestinationRoot){$DestinationRoot=$defaultDestination}
function Fail([string]$m){throw "Live2D asset staging failed: $m"}
function Hash([string]$p){(Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLowerInvariant()}
function Test-PathContains([string]$Parent,[string]$Child){$p=[IO.Path]::GetFullPath($Parent).TrimEnd('\');$c=[IO.Path]::GetFullPath($Child).TrimEnd('\');[string]::Equals($p,$c,[StringComparison]::OrdinalIgnoreCase)-or$c.StartsWith($p+'\',[StringComparison]::OrdinalIgnoreCase)}
function Assert-NoReparseAncestor([string]$Path){
    $cursor=[IO.Path]::GetFullPath($Path)
    while($cursor){
        if(Test-Path -LiteralPath $cursor){$item=Get-Item -Force -LiteralPath $cursor;if(($item.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne 0){Fail "reparse ancestor forbidden: $cursor"}}
        $parent=Split-Path -Parent $cursor;if($parent -eq $cursor){break};$cursor=$parent
    }
}
function Assert-SafeDestination([string]$Path){
    $full=[IO.Path]::GetFullPath($Path).TrimEnd('\')
    $allowed=$full -eq $defaultDestination
    if(-not $allowed){
        $temp=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\'
        if($full.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)){
            $relative=$full.Substring($temp.Length);$first=($relative -split '[\\/]')[0]
            $allowed=$first -like 'parallel-world-live2d-*'
        }
    }
    if(-not $allowed -or $full -eq $repoRoot -or $full -eq [IO.Path]::GetPathRoot($full)){Fail 'DestinationRoot is outside the fixed dev stage or approved isolated test root.'}
    Assert-NoReparseAncestor $full
    $full
}
function Resolve-ExactDirectory([string]$Root,[string]$Relative){
    $current=Get-Item -Force -LiteralPath $Root
    foreach($segment in @($Relative -split '[\\/]'|Where-Object{$_})){
        if(($current.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne 0){Fail 'source reparse point forbidden.'}
        $match=@(Get-ChildItem -Directory -Force -LiteralPath $current.FullName|Where-Object{[string]::Equals($_.Name,$segment,[StringComparison]::Ordinal)})
        if($match.Count-ne 1){Fail "missing directory or case mismatch: $Relative"};$current=$match[0]
    };$current
}
function Copy-Declared([string]$Source,[string]$Destination,[string]$ReceiptPath,[int64]$ExpectedSize,[string]$ExpectedHash,[Collections.ArrayList]$Receipt){
    $before=Get-Item -Force -LiteralPath $Source
    if($before.Length-ne $ExpectedSize -or (Hash $Source)-ne $ExpectedHash){Fail "source changed before copy: $ReceiptPath"}
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination)|Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination
    if((Get-Item -LiteralPath $Destination).Length-ne $ExpectedSize -or (Hash $Destination)-ne $ExpectedHash){Fail "destination differs from declared bytes: $ReceiptPath"}
    if((Get-Item -LiteralPath $Source).Length-ne $ExpectedSize -or (Hash $Source)-ne $ExpectedHash){Fail "source changed during copy: $ReceiptPath"}
    [void]$Receipt.Add([ordered]@{path=$ReceiptPath.Replace('\','/');size=$ExpectedSize;sha256=$ExpectedHash})
}
try{
    Import-Module (Join-Path $PSScriptRoot 'Live2DManifest.psm1') -Force
    $source=Get-Item -Force -LiteralPath $SourceRoot;if(-not $source.PSIsContainer){Fail 'SourceRoot must be a directory.'}
    $destination=Assert-SafeDestination $DestinationRoot
    $sourceFull=$source.FullName.TrimEnd('\')
    $isFixedDestination=$destination -eq $defaultDestination
    if((Test-PathContains $destination $sourceFull) -or ((-not $isFixedDestination) -and (Test-PathContains $sourceFull $destination))){Fail 'SourceRoot and destination must not equal or dangerously contain each other.'}
    $manifestFull=[IO.Path]::GetFullPath($ManifestPath)
    if((Test-PathContains $destination $manifestFull)-or(Test-PathContains $manifestFull $destination)){Fail 'ManifestPath and destination must not contain each other.'}
    $models=@();foreach($v in @($Model)){$models+=@($v -split ','|Where-Object{$_})};if(-not $models.Count){Fail 'model required.'}
    $ids=@('live2d-cubism-core')+$models
    $parsed=Read-Live2DManifest $ManifestPath
    $assets=Test-Live2DManifestAssets $parsed.Document $source.FullName $ids
    $frameworkRelative=(Get-Live2DSafeSegments $FrameworkRelativePath 'FrameworkRelativePath')-join '\'
    $framework=Resolve-ExactDirectory $source.FullName $frameworkRelative
    Assert-Live2DNoReparseAncestors $framework.FullName
    Assert-Live2DNoNestedReparse $framework.FullName
    if((Test-PathContains $destination $framework.FullName)-or(Test-PathContains $framework.FullName $destination)){Fail 'Framework and destination must not contain each other.'}
    $frameworkFiles=@(Get-ChildItem -Recurse -File -Force -LiteralPath $framework.FullName|ForEach-Object{
        if(($_.Attributes-band[IO.FileAttributes]::ReparsePoint)-ne 0){Fail 'Framework reparse point forbidden.'}
        [ordered]@{source=$_.FullName;relative=$_.FullName.Substring($framework.FullName.Length).TrimStart('\','/');size=$_.Length;sha256=(Hash $_.FullName)}
    });if(-not $frameworkFiles.Count){Fail 'Framework is empty.'}
    $parent=Split-Path -Parent $destination;New-Item -ItemType Directory -Force -Path $parent|Out-Null;Assert-NoReparseAncestor $parent
    $leaf=Split-Path -Leaf $destination;$temporary=Join-Path $parent "$leaf.staging-$([guid]::NewGuid().ToString('N'))";$backup=Join-Path $parent "$leaf.backup-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $temporary|Out-Null;$receipt=[Collections.ArrayList]::new();$backedUp=$false
    try{
        foreach($asset in $assets){$assetSource=Join-Path $source.FullName $asset.localPath;$base=if($asset.id-eq'live2d-cubism-core'){'core'}else{"models/$($asset.id)"};foreach($file in $asset.files){$rel=$file.path.Replace('/','\');Copy-Declared (Join-Path $assetSource $rel) (Join-Path $temporary "$base\$rel") "$base/$($file.path)" ([int64]$file.size) $file.sha256.ToLowerInvariant() $receipt}}
        foreach($file in $frameworkFiles){Copy-Declared $file.source (Join-Path $temporary "framework\$($file.relative)") "framework/$($file.relative)" $file.size $file.sha256 $receipt}
        if((Hash $ManifestPath)-ne $parsed.RawHash){Fail 'manifest changed during staging.'}
        $doc=[ordered]@{schemaVersion=1;generatedAt=[DateTime]::UtcNow.ToString('o');files=@($receipt|Sort-Object path)}
        [IO.File]::WriteAllText((Join-Path $temporary 'staging-manifest.json'),($doc|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
        foreach($entry in $receipt){$p=Join-Path $temporary $entry.path.Replace('/','\');if((Hash $p)-ne $entry.sha256){Fail "final hash mismatch: $($entry.path)"}}
        Assert-NoReparseAncestor $destination
        if(Test-Path -LiteralPath $destination){Move-Item -LiteralPath $destination -Destination $backup;$backedUp=$true}
        if($env:PW_LIVE2D_TEST_FAIL_SWAP -eq '1'){Fail 'simulated swap failure.'}
        Move-Item -LiteralPath $temporary -Destination $destination;$temporary=$null
        if($backedUp){Remove-Item -Recurse -Force -LiteralPath $backup;$backedUp=$false}
    }catch{
        if($backedUp -and -not(Test-Path -LiteralPath $destination)){Move-Item -LiteralPath $backup -Destination $destination;$backedUp=$false};throw
    }finally{
        if($temporary -and(Test-Path -LiteralPath $temporary)){Remove-Item -Recurse -Force -LiteralPath $temporary}
        if($backedUp -and(Test-Path -LiteralPath $backup)){Remove-Item -Recurse -Force -LiteralPath $backup}
    }
    [ordered]@{destination=$destination;models=$models;fileCount=$receipt.Count}|ConvertTo-Json;exit 0
}catch{[Console]::Error.WriteLine($_.Exception.Message);exit 1}
