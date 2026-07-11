$ErrorActionPreference = 'Stop'

$toolsRoot = Split-Path -Parent $PSScriptRoot
$validator = Join-Path $toolsRoot 'Test-Live2DManifest.ps1'
$stager = Join-Path $toolsRoot 'stage-dev-assets.ps1'
$script:failures = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        $script:failures++
        Write-Host "FAIL: $Message" -ForegroundColor Red
    } else {
        Write-Host "PASS: $Message" -ForegroundColor Green
    }
}

function Invoke-ChildPowerShell {
    param([string]$ScriptPath, [string[]]$Arguments)
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ScriptPath @Arguments 2>&1
    $ErrorActionPreference = $previousPreference
    [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = ($output -join "`n") }
}

function New-Fixture {
    param([string]$Case)
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("parallel-world-live2d-{0}" -f [guid]::NewGuid())
    $source = Join-Path $root 'source'
    $manifestDir = Join-Path $root 'manifest'
    New-Item -ItemType Directory -Force -Path (Join-Path $source 'core'), (Join-Path $source 'models/mark'), $manifestDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $source 'core/core.js'), 'core', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $source 'models/mark/model.json'), 'model', [Text.UTF8Encoding]::new($false))
    $coreHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $source 'core/core.js')).Hash.ToLowerInvariant()
    $modelHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $source 'models/mark/model.json')).Hash.ToLowerInvariant()
    $corePath = 'core.js'
    $coreLocalPath = 'core'
    switch ($Case) {
        'hash-mismatch' { $coreHash = ('0' * 64) }
        'traversal' { $corePath = '../escape.js' }
        'absolute' { $corePath = (Join-Path $source 'core/core.js') }
        'case-mismatch' { $corePath = 'Core.js' }
        'missing-core' { Remove-Item -LiteralPath (Join-Path $source 'core/core.js') }
        'undeclared' { [IO.File]::WriteAllText((Join-Path $source 'core/extra.js'), 'extra', [Text.UTF8Encoding]::new($false)) }
    }
    $manifest = [ordered]@{
        schemaVersion = 1
        generatedAt = '2026-07-11T00:00:00Z'
        assets = @(
            [ordered]@{ id='live2d-cubism-core'; name='Core'; sourceType='fixture'; sourceUrl='https://example.invalid/core'; sourceVersion='1'; localPath=$coreLocalPath; licenseCategory='fixture'; intendedUse=@('test'); redistributionApproved=$false; files=@([ordered]@{path=$corePath; size=4; sha256=$coreHash}) },
            [ordered]@{ id='live2d-mark'; name='Mark'; sourceType='fixture'; sourceUrl='https://example.invalid/mark'; sourceVersion='1'; localPath='models/mark'; licenseCategory='fixture'; intendedUse=@('test'); redistributionApproved=$false; files=@([ordered]@{path='model.json'; size=5; sha256=$modelHash}) }
        )
    }
    $manifestPath = Join-Path $manifestDir 'assets.json'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    [pscustomobject]@{ Root=$root; Source=$source; Manifest=$manifestPath }
}

foreach ($case in @('missing-core','hash-mismatch','traversal','absolute','case-mismatch','undeclared')) {
    $fixture = New-Fixture $case
    try {
        $result = Invoke-ChildPowerShell $validator @('-ManifestPath',$fixture.Manifest,'-SourceRoot',$fixture.Source,'-AssetId','live2d-cubism-core,live2d-mark')
        Assert-True ($result.ExitCode -ne 0) "$case is rejected"
    } finally { Remove-Item -Recurse -Force -LiteralPath $fixture.Root }
}

$schemaCases = @(
    @{ Name='unknown-root-field'; Mutate={ param($m) $m|Add-Member -NotePropertyName bad -NotePropertyValue x } },
    @{ Name='duplicate-id'; Mutate={ param($m) $m.assets[1].id=$m.assets[0].id } },
    @{ Name='invalid-size'; Mutate={ param($m) $m.assets[0].files[0].size=-1 } },
    @{ Name='invalid-hash'; Mutate={ param($m) $m.assets[0].files[0].sha256='xyz' } },
    @{ Name='empty-files'; Mutate={ param($m) $m.assets[0].files=@() } },
    @{ Name='missing-redistribution'; Mutate={ param($m) $m.assets[0].PSObject.Properties.Remove('redistributionApproved') } },
    @{ Name='string-redistribution'; Mutate={ param($m) $m.assets[0].redistributionApproved='false' } },
    @{ Name='missing-source-type'; Mutate={ param($m) $m.assets[0].PSObject.Properties.Remove('sourceType') } }
)
foreach ($schemaCase in $schemaCases) {
    $fixture = New-Fixture 'valid'
    try {
        $document=Get-Content -Raw $fixture.Manifest|ConvertFrom-Json
        & $schemaCase.Mutate $document
        [IO.File]::WriteAllText($fixture.Manifest,($document|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        $result=Invoke-ChildPowerShell $validator @('-ManifestPath',$fixture.Manifest,'-SourceRoot',$fixture.Source)
        Assert-True ($result.ExitCode -ne 0) "$($schemaCase.Name) schema is rejected"
    } finally { Remove-Item -Recurse -Force -LiteralPath $fixture.Root }
}

$valid = New-Fixture 'valid'
try {
    $result = Invoke-ChildPowerShell $validator @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-AssetId','live2d-cubism-core,live2d-mark')
    if ($result.ExitCode -ne 0) { Write-Host $result.Output }
    Assert-True ($result.ExitCode -eq 0) 'valid manifest is accepted'

    $framework = Join-Path $valid.Source 'framework'
    New-Item -ItemType Directory -Force -Path $framework | Out-Null
    [IO.File]::WriteAllText((Join-Path $framework 'framework.js'), 'framework', [Text.UTF8Encoding]::new($false))
    $outside=Join-Path $valid.Root 'outside-empty';New-Item -ItemType Directory -Path $outside|Out-Null
    New-Item -ItemType Junction -Path (Join-Path $valid.Source 'core/nested-link') -Target $outside|Out-Null
    $nestedAssetResult=Invoke-ChildPowerShell $validator @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-AssetId','live2d-cubism-core')
    Assert-True ($nestedAssetResult.ExitCode -ne 0) 'nested asset directory reparse point is rejected'
    Remove-Item -LiteralPath (Join-Path $valid.Source 'core/nested-link')
    New-Item -ItemType Junction -Path (Join-Path $framework 'nested-link') -Target $outside|Out-Null
    $nestedFrameworkResult=Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',(Join-Path $valid.Root 'nested-stage'),'-Model','live2d-mark','-FrameworkRelativePath','framework')
    Assert-True ($nestedFrameworkResult.ExitCode -ne 0) 'nested Framework directory reparse point is rejected'
    Remove-Item -LiteralPath (Join-Path $framework 'nested-link')
    $destination = Join-Path $valid.Root 'staged'
    [IO.File]::WriteAllText((Join-Path $valid.Source 'sentinel.txt'),'keep')
    $sameResult=Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',$valid.Source,'-Model','live2d-mark','-FrameworkRelativePath','framework')
    Assert-True ($sameResult.ExitCode -ne 0) 'source equal to destination is rejected'
    Assert-True (Test-Path (Join-Path $valid.Source 'sentinel.txt')) 'source sentinel survives equal destination rejection'
    Remove-Item -LiteralPath (Join-Path $valid.Source 'sentinel.txt') -ErrorAction SilentlyContinue
    $unsafeDestination = Join-Path $valid.Source 'destructive-target'
    New-Item -ItemType Directory -Force -Path $unsafeDestination | Out-Null
    [IO.File]::WriteAllText((Join-Path $unsafeDestination 'sentinel.txt'),'keep')
    $unsafeResult=Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',$unsafeDestination,'-Model','live2d-mark','-FrameworkRelativePath','framework')
    Assert-True ($unsafeResult.ExitCode -ne 0) 'destination under source is rejected'
    Assert-True (Test-Path (Join-Path $unsafeDestination 'sentinel.txt')) 'unsafe destination is never deleted'
    $caseResult = Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',$destination,'-Model','live2d-mark','-FrameworkRelativePath','Framework')
    Assert-True ($caseResult.ExitCode -ne 0) 'Framework path case mismatch is rejected'
    $result = Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',$destination,'-Model','live2d-mark','-FrameworkRelativePath','framework')
    if ($result.ExitCode -ne 0) { Write-Host $result.Output }
    Assert-True ($result.ExitCode -eq 0) 'valid synthetic assets are staged'
    Assert-True (Test-Path -LiteralPath (Join-Path $destination 'core/core.js')) 'Core keeps its relative layout'
    Assert-True (Test-Path -LiteralPath (Join-Path $destination 'framework/framework.js')) 'Framework is staged'
    Assert-True (Test-Path -LiteralPath (Join-Path $destination 'models/live2d-mark/model.json')) 'model keeps its relative layout'
    Assert-True (Test-Path -LiteralPath (Join-Path $destination 'staging-manifest.json')) 'copy hash receipt is emitted'
    $receiptDocument=Get-Content -Raw (Join-Path $destination 'staging-manifest.json')|ConvertFrom-Json
    Assert-True (-not ($receiptDocument.PSObject.Properties.Name -contains 'sourceRoot')) 'receipt contains no absolute sourceRoot'
    [IO.File]::WriteAllText((Join-Path $destination 'old-stage.txt'),'preserve')
    $env:PW_LIVE2D_TEST_FAIL_SWAP='1'
    $rollbackResult=Invoke-ChildPowerShell $stager @('-ManifestPath',$valid.Manifest,'-SourceRoot',$valid.Source,'-DestinationRoot',$destination,'-Model','live2d-mark','-FrameworkRelativePath','framework')
    Remove-Item Env:PW_LIVE2D_TEST_FAIL_SWAP
    Assert-True ($rollbackResult.ExitCode -ne 0) 'swap failure is reported'
    Assert-True (Test-Path (Join-Path $destination 'old-stage.txt')) 'old stage is restored after swap failure'
} finally { Remove-Item -Recurse -Force -LiteralPath $valid.Root }

if ($script:failures -gt 0) { throw "$script:failures Live2D asset test(s) failed." }
Write-Host 'All Live2D asset tests passed.' -ForegroundColor Green
