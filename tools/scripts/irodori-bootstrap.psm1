Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$AllowedManifestFields = @('schema_version', 'manifest_version', 'python_version', 'backends', 'artifacts')
$AllowedArtifactFields = @('id', 'url', 'size', 'sha256', 'install_relative_path', 'license_id', 'license_url')
$AllowedLicenseIds = @('Apache-2.0 OR MIT', 'MIT')
$AllowedBackends = @('cpu', 'cu128')

function Assert-IrodoriExactFields {
    param([psobject] $Value, [string[]] $ExpectedFields, [string] $Context)

    $actualFields = @($Value.PSObject.Properties.Name)
    $missingFields = @($ExpectedFields | Where-Object { $_ -notin $actualFields })
    $unknownFields = @($actualFields | Where-Object { $_ -notin $ExpectedFields })
    if ($missingFields.Count -gt 0 -or $unknownFields.Count -gt 0) {
        throw "$Context has missing or unknown fields. Missing: $($missingFields -join ', '); unknown: $($unknownFields -join ', ')."
    }
}

function Test-IrodoriRelativePath {
    param([string] $Path)
    if ([string]::IsNullOrWhiteSpace($Path) -or [IO.Path]::IsPathRooted($Path)) { return $false }
    return @($Path -split '[\\/]' | Where-Object { $_ -in @('', '.', '..') }).Count -eq 0
}

function Test-IrodoriHttpsUrl {
    param([string] $Url)
    $uri = $null
    return [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref] $uri) -and $uri.Scheme -eq 'https' -and -not [string]::IsNullOrWhiteSpace($uri.Host)
}

function Import-IrodoriManifest {
    param([string] $Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Irodori manifest does not exist: $Path" }
    try { $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -ErrorAction Stop } catch { throw "Irodori manifest is not valid JSON: $Path" }

    Assert-IrodoriExactFields $manifest $AllowedManifestFields 'Irodori manifest'
    if ($manifest.schema_version -ne 1) { throw 'Irodori manifest schema_version must be 1.' }
    if ($manifest.manifest_version -notmatch '^\d{4}-\d{2}-\d{2}\.\d+$') { throw 'Irodori manifest_version must be a versioned date.' }
    if ($manifest.python_version -notmatch '^3\.10\.\d+$') { throw 'Irodori python_version must be a CPython 3.10 patch version.' }
    if (@($manifest.backends).Count -ne $AllowedBackends.Count -or (@($manifest.backends | Sort-Object -Unique) -join ',') -ne ($AllowedBackends -join ',')) { throw 'Irodori backends must be exactly cpu and cu128.' }
    if (@($manifest.artifacts).Count -eq 0) { throw 'Irodori manifest must contain artifacts.' }

    $artifactIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $installPaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($artifact in @($manifest.artifacts)) {
        Assert-IrodoriExactFields $artifact $AllowedArtifactFields 'Irodori artifact'
        if ([string]::IsNullOrWhiteSpace($artifact.id) -or -not $artifactIds.Add($artifact.id)) { throw "Irodori artifact id must be unique: $($artifact.id)" }
        if (-not (Test-IrodoriHttpsUrl $artifact.url)) { throw "Irodori artifact URL must use HTTPS: $($artifact.id)" }
        if ($artifact.size -isnot [ValueType] -or [int64]$artifact.size -le 0) { throw "Irodori artifact size must be positive: $($artifact.id)" }
        if ($artifact.sha256 -notmatch '^[0-9a-f]{64}$') { throw "Irodori artifact sha256 must be lowercase 64-hex: $($artifact.id)" }
        if (-not (Test-IrodoriRelativePath $artifact.install_relative_path) -or -not $installPaths.Add($artifact.install_relative_path)) { throw "Irodori artifact install_relative_path must be unique and relative: $($artifact.id)" }
        if ($artifact.license_id -notin $AllowedLicenseIds) { throw "Irodori artifact license_id is not supported: $($artifact.id)" }
        if (-not (Test-IrodoriHttpsUrl $artifact.license_url)) { throw "Irodori artifact license_url must use HTTPS: $($artifact.id)" }
    }
    return $manifest
}

function Get-IrodoriLayout {
    param(
        [string] $Root = (Join-Path $env:LOCALAPPDATA 'com.parallelworld.desktop\irodori'),
        [string] $ManifestVersion
    )
    if ([string]::IsNullOrWhiteSpace($Root)) { throw 'Irodori layout root must not be empty.' }
    if ($ManifestVersion -notmatch '^\d{4}-\d{2}-\d{2}\.\d+$') { throw 'Irodori layout manifest version must be a versioned date.' }
    $runtimeRoot = Join-Path $Root 'runtime'
    $runtime = Join-Path $runtimeRoot $ManifestVersion
    $cacheRoot = Join-Path $Root 'cache'
    $userRoot = Join-Path $Root 'user'
    return @{
        root = $Root; runtime_root = $runtimeRoot; runtime = $runtime; cache_root = $cacheRoot
        downloads = Join-Path $cacheRoot 'downloads'; transactions = Join-Path $Root 'transactions'
        user_root = $userRoot; voices = Join-Path $userRoot 'voices'; loras = Join-Path $userRoot 'loras'
        completion_marker = Join-Path $runtime 'completion.json'; active_marker = Join-Path $runtimeRoot 'active.json'
    }
}

function Get-IrodoriBackend {
    param([string[]] $GpuNames)
    if (@($GpuNames) | Where-Object { $_ -match '(?i)NVIDIA' }) { return 'cu128' }
    return 'cpu'
}

function Test-IrodoriCompletion {
    param([hashtable] $Layout, [psobject] $Manifest)
    if (-not $Layout.ContainsKey('completion_marker') -or -not (Test-Path -LiteralPath $Layout.completion_marker -PathType Leaf)) { return $false }
    try {
        $completion = Get-Content -LiteralPath $Layout.completion_marker -Raw | ConvertFrom-Json -ErrorAction Stop
        Assert-IrodoriExactFields $completion @('schema_version', 'manifest_version', 'backend', 'python_version', 'completed_at') 'Irodori completion marker'
        [DateTimeOffset]::Parse($completion.completed_at) | Out-Null
    } catch { return $false }
    return $completion.schema_version -eq 1 -and $completion.manifest_version -eq $Manifest.manifest_version -and $completion.backend -in @($Manifest.backends) -and $completion.python_version -eq $Manifest.python_version
}

Export-ModuleMember -Function Import-IrodoriManifest, Get-IrodoriLayout, Get-IrodoriBackend, Test-IrodoriCompletion
