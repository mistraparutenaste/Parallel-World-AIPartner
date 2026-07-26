[CmdletBinding()]
param(
    [switch] $SkipOptionalModels
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $repositoryRoot

function Confirm-Download {
    param(
        [Parameter(Mandatory)] [string] $Description,
        [Parameter(Mandatory)] [string] $Size
    )

    while ($true) {
        $answer = (Read-Host "$Description requires a download ($Size). Continue? [y/n]").Trim()
        if ($answer -match '^(?i:y|yes)$') { return $true }
        if ($answer -match '^(?i:n|no)$') { return $false }
        Write-Host 'Please enter y or n.' -ForegroundColor Yellow
    }
}

function Refresh-ProcessPath {
    $machinePath = [Environment]::GetEnvironmentVariable('PATH', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
    $cargoPath = Join-Path $env:USERPROFILE '.cargo\bin'
    $env:PATH = (@($machinePath, $userPath, $cargoPath) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join [IO.Path]::PathSeparator
}

function Test-Command {
    param([Parameter(Mandatory)] [string] $Name)
    return $null -ne (Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue)
}

function Invoke-WingetInstall {
    param(
        [Parameter(Mandatory)] [string] $Id,
        [string[]] $AdditionalArguments = @()
    )

    if (-not (Test-Command 'winget.exe')) {
        throw 'Windows Package Manager (winget) is required. Install App Installer from Microsoft Store, then run this launcher again.'
    }

    $arguments = @(
        'install', '--id', $Id, '--exact', '--silent',
        '--accept-package-agreements', '--accept-source-agreements',
        '--disable-interactivity'
    ) + $AdditionalArguments
    & winget.exe @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "winget could not install $Id (exit code $LASTEXITCODE)."
    }
    Refresh-ProcessPath
}

function Test-MsvcBuildTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) { return $false }
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    return -not [string]::IsNullOrWhiteSpace(($installation | Select-Object -First 1))
}

function Test-WebView2Runtime {
    $productId = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    foreach ($scope in @('HKLM:', 'HKCU:')) {
        foreach ($architecture in @('', '\Wow6432Node')) {
            $path = "$scope\SOFTWARE$architecture\Microsoft\EdgeUpdate\Clients\$productId"
            $version = Get-ItemPropertyValue -LiteralPath $path -Name 'pv' -ErrorAction SilentlyContinue
            if (-not [string]::IsNullOrWhiteSpace($version) -and $version -ne '0.0.0.0') {
                return $true
            }
        }
    }
    return $false
}

function Assert-NodeVersion {
    $raw = (& node --version).TrimStart('v')
    $parts = @($raw.Split('.') | ForEach-Object { [int] $_ })
    if ($parts.Count -lt 3 -or
        $parts[0] -lt 24 -or
        ($parts[0] -eq 24 -and $parts[1] -lt 15)) {
        throw "Node.js 24.15.0 or newer is required. Detected: $raw"
    }
}

Write-Host ''
Write-Host '===== Parallel World: environment preparation =====' -ForegroundColor Cyan

if (-not (Test-Command 'node.exe')) {
    if (-not (Confirm-Download -Description 'Node.js' -Size 'about 40 MB')) {
        throw 'Node.js installation was declined. The app cannot start without Node.js.'
    }
    Invoke-WingetInstall -Id 'OpenJS.NodeJS'
}
Assert-NodeVersion

if (-not (Test-Command 'cargo.exe')) {
    if (-not (Confirm-Download -Description 'Rust and Cargo' -Size 'about 500 MB after toolchain installation')) {
        throw 'Rust installation was declined. The app cannot start without Rust.'
    }
    Invoke-WingetInstall -Id 'Rustlang.Rustup'
    Refresh-ProcessPath
    & rustup.exe toolchain install stable
    if ($LASTEXITCODE -ne 0) { throw 'Rust stable toolchain installation failed.' }
    & rustup.exe default stable
    if ($LASTEXITCODE -ne 0) { throw 'Rust stable toolchain selection failed.' }
}

if (-not (Test-MsvcBuildTools)) {
    if (-not (Confirm-Download -Description 'Microsoft C++ Build Tools' -Size 'several GB')) {
        throw 'Microsoft C++ Build Tools installation was declined. The Windows app cannot be compiled without it.'
    }
    Invoke-WingetInstall -Id 'Microsoft.VisualStudio.2022.BuildTools' -AdditionalArguments @(
        '--override',
        '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    )
}

if (-not (Test-WebView2Runtime)) {
    if (-not (Confirm-Download -Description 'Microsoft Edge WebView2 Runtime' -Size 'about 200 MB')) {
        throw 'WebView2 Runtime installation was declined. The desktop UI cannot start without it.'
    }
    Invoke-WingetInstall -Id 'Microsoft.EdgeWebView2Runtime'
}

if (-not (Test-Command 'corepack.cmd')) {
    throw 'Corepack was not installed with Node.js. Repair the Node.js installation and run this launcher again.'
}

if (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules') -PathType Container)) {
    if (-not (Confirm-Download -Description 'JavaScript dependencies' -Size 'several hundred MB')) {
        throw 'JavaScript dependency installation was declined. The app cannot start without these packages.'
    }
    & corepack.cmd pnpm install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'JavaScript dependency installation failed.' }
}

$live2dDist = Join-Path $repositoryRoot 'packages\live2d-runtime\dist'
if (-not (Test-Path -LiteralPath $live2dDist -PathType Container)) {
    Write-Host '[Setup] Building workspace packages.' -ForegroundColor Cyan
    & corepack.cmd pnpm build
    if ($LASTEXITCODE -ne 0) { throw 'Workspace build failed.' }
}

Write-Host '[Setup] Synchronizing available Live2D development assets.' -ForegroundColor Cyan
& node.exe tools/scripts/sync-live2d-dev-assets.mjs
if ($LASTEXITCODE -ne 0) { throw 'Live2D development asset synchronization failed.' }

if (-not $SkipOptionalModels) {
    $modelRoot = Join-Path $env:APPDATA 'com.parallelworld.desktop\models'
    $hasSpeechModel = (Test-Path -LiteralPath $modelRoot) -and
        ($null -ne (Get-ChildItem -LiteralPath $modelRoot -Recurse -Filter '*.onnx' -ErrorAction SilentlyContinue | Select-Object -First 1))
    if (-not $hasSpeechModel) {
        if (Confirm-Download -Description 'Basic speech recognition models' -Size 'about 716 MB') {
            & node.exe tools/scripts/download-stt-models.mjs
            if ($LASTEXITCODE -ne 0) { throw 'Speech recognition model installation failed.' }
        } else {
            Write-Host '[Setup] Speech recognition models were skipped. Text chat will still work.' -ForegroundColor Yellow
        }
    }
}

Write-Host '[Setup] Environment preparation is complete.' -ForegroundColor Green
