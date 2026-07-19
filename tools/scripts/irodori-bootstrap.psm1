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

function Test-IrodoriString {
    param([object] $Value)
    return $Value -is [string]
}

function Test-IrodoriPositiveInteger {
    param([object] $Value)
    if ($Value -isnot [byte] -and $Value -isnot [sbyte] -and $Value -isnot [int16] -and $Value -isnot [uint16] -and $Value -isnot [int32] -and $Value -isnot [uint32] -and $Value -isnot [int64] -and $Value -isnot [uint64]) {
        return $false
    }
    return $Value -gt 0
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
        $artifactBaseName = if (Test-IrodoriString $artifact.id) { ($artifact.id -split '\.')[0] } else { '' }
        if (-not (Test-IrodoriString $artifact.id) -or $artifact.id -cnotmatch '^[a-z0-9][a-z0-9._-]*$' -or $artifact.id -in @('.', '..') -or $artifactBaseName -match '^(?i:con|prn|aux|nul|clock\$|com[1-9]|lpt[1-9])$' -or -not $artifactIds.Add($artifact.id)) { throw "Irodori artifact id must be a unique safe lowercase basename: $($artifact.id)" }
        if (-not (Test-IrodoriString $artifact.url) -or -not (Test-IrodoriHttpsUrl $artifact.url)) { throw "Irodori artifact URL must be an HTTPS string: $($artifact.id)" }
        if (-not (Test-IrodoriPositiveInteger $artifact.size)) { throw "Irodori artifact size must be a positive integer: $($artifact.id)" }
        if (-not (Test-IrodoriString $artifact.sha256) -or $artifact.sha256 -notmatch '^[0-9a-f]{64}$') { throw "Irodori artifact sha256 must be a lowercase 64-hex string: $($artifact.id)" }
        if (-not (Test-IrodoriString $artifact.install_relative_path) -or -not (Test-IrodoriRelativePath $artifact.install_relative_path) -or -not $installPaths.Add($artifact.install_relative_path)) { throw "Irodori artifact install_relative_path must be a unique relative string: $($artifact.id)" }
        if (-not (Test-IrodoriString $artifact.license_id) -or $artifact.license_id -notin $AllowedLicenseIds) { throw "Irodori artifact license_id is not supported: $($artifact.id)" }
        if (-not (Test-IrodoriString $artifact.license_url) -or -not (Test-IrodoriHttpsUrl $artifact.license_url)) { throw "Irodori artifact license_url must be an HTTPS string: $($artifact.id)" }
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
    param(
        [hashtable] $Layout,
        [psobject] $Manifest,
        [Parameter(Mandatory)] [ValidateSet('cpu', 'cu128')] [string] $ExpectedBackend
    )
    if (-not $Layout.ContainsKey('completion_marker') -or -not (Test-Path -LiteralPath $Layout.completion_marker -PathType Leaf)) { return $false }
    try {
        $completion = Get-Content -LiteralPath $Layout.completion_marker -Raw | ConvertFrom-Json -ErrorAction Stop
        Assert-IrodoriExactFields $completion @('schema_version', 'manifest_version', 'backend', 'python_version', 'completed_at') 'Irodori completion marker'
        [DateTimeOffset]::Parse($completion.completed_at) | Out-Null
    } catch { return $false }
    return $completion.schema_version -eq 1 -and $completion.manifest_version -eq $Manifest.manifest_version -and $completion.backend -in @($Manifest.backends) -and $completion.backend -eq $ExpectedBackend -and $completion.python_version -eq $Manifest.python_version
}

function Get-IrodoriCanonicalPath {
    param([string] $Path)
    return [IO.Path]::GetFullPath($Path).TrimEnd([char[]] @([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar))
}

function Test-IrodoriDescendantPath {
    param([string] $Root, [string] $Candidate)
    $canonicalRoot = Get-IrodoriCanonicalPath $Root
    $canonicalCandidate = Get-IrodoriCanonicalPath $Candidate
    $prefix = $canonicalRoot + [IO.Path]::DirectorySeparatorChar
    return $canonicalCandidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Assert-IrodoriCleanupTarget {
    param([string] $Root, [string] $Target)
    if (-not (Test-IrodoriDescendantPath $Root $Target)) { throw 'Irodori cleanup target escapes the managed root.' }
    $canonicalRoot = Get-IrodoriCanonicalPath $Root
    if (Test-Path -LiteralPath $canonicalRoot) {
        $rootItem = Get-Item -LiteralPath $canonicalRoot -Force
        if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Irodori managed root must not be a reparse point.' }
    }
    $cursor = Get-IrodoriCanonicalPath $Target
    while ($cursor.StartsWith($canonicalRoot, [StringComparison]::OrdinalIgnoreCase) -and $cursor -ne $canonicalRoot) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Irodori cleanup target contains a reparse point.' }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ([string]::IsNullOrEmpty($parent) -or $parent -eq $cursor) { break }
        $cursor = $parent
    }
}

function Remove-IrodoriManagedItem {
    param([hashtable] $Layout, [string] $Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $Path
    Remove-Item -LiteralPath $Path -Force -Recurse
}

function Write-IrodoriAtomicText {
    param([hashtable] $Layout, [string] $Path, [string] $Text)
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $Path
    $directory = [IO.Path]::GetDirectoryName($Path)
    [void] [IO.Directory]::CreateDirectory($directory)
    $temporary = Join-Path $directory ('.atomic-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $encoding = [Text.UTF8Encoding]::new($false)
        $stream = [IO.FileStream]::new($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $bytes = $encoding.GetBytes($Text)
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush($true)
        } finally { $stream.Dispose() }
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $replaceBackup = Join-Path $directory ('.atomic-backup-' + [Guid]::NewGuid().ToString('N') + '.tmp')
            try { [IO.File]::Replace($temporary, $Path, $replaceBackup) } finally {
                if (Test-Path -LiteralPath $replaceBackup) { Remove-Item -LiteralPath $replaceBackup -Force }
            }
        } else {
            [IO.File]::Move($temporary, $Path)
        }
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Write-IrodoriJson {
    param([hashtable] $Layout, [string] $Path, [object] $Value)
    Write-IrodoriAtomicText -Layout $Layout -Path $Path -Text ($Value | ConvertTo-Json -Depth 10 -Compress)
}

function Test-IrodoriVerifiedFile {
    param([string] $Path, [psobject] $Artifact)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $item.Length -ne [int64] $Artifact.size) { return $false }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $Artifact.sha256
}

function Invoke-IrodoriHttpDownload {
    param([psobject] $Artifact, [string] $PartialPath, [int64] $MaximumBytes)
    if (-not (Test-IrodoriHttpsUrl $Artifact.url)) { throw 'Irodori download URL must use HTTPS.' }
    Add-Type -AssemblyName System.Net.Http
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $handler.UseDefaultCredentials = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $current = [Uri] $Artifact.url
    try {
        for ($redirects = 0; $redirects -le 10; $redirects++) {
            if ($current.Scheme -ne 'https') { throw 'Irodori download redirect must use HTTPS.' }
            $response = $client.GetAsync($current, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            try {
                if ([int] $response.StatusCode -ge 300 -and [int] $response.StatusCode -lt 400) {
                    if ($redirects -eq 10 -or $null -eq $response.Headers.Location) { throw 'Irodori download redirect is invalid or excessive.' }
                    $current = [Uri]::new($current, $response.Headers.Location)
                    if ($current.Scheme -ne 'https') { throw 'Irodori download redirect must use HTTPS.' }
                    continue
                }
                $response.EnsureSuccessStatusCode() | Out-Null
                if ($response.Content.Headers.ContentLength.HasValue -and $response.Content.Headers.ContentLength.Value -gt $MaximumBytes) { throw 'Irodori download exceeds the manifest size.' }
                $input = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $output = [IO.FileStream]::new($PartialPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None, 1048576, [IO.FileOptions]::WriteThrough)
                try {
                    $buffer = [byte[]]::new(1048576)
                    [int64] $total = 0
                    while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        if ($total + $read -gt $MaximumBytes) { throw 'Irodori download exceeds the manifest size.' }
                        $output.Write($buffer, 0, $read)
                        $total += $read
                    }
                    $output.Flush($true)
                } finally {
                    $output.Dispose()
                    $input.Dispose()
                }
                return [pscustomobject]@{ final_url = $current.AbsoluteUri; bytes_written = $total; cancelled = $false }
            } finally { $response.Dispose() }
        }
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
    throw 'Irodori download failed.'
}

function Get-IrodoriArtifactCachePath {
    param([hashtable] $Layout, [psobject] $Artifact)
    return Join-Path $Layout.downloads ($Artifact.id + '.artifact')
}

function Get-IrodoriVerifiedArtifact {
    param([hashtable] $Layout, [psobject] $Artifact, [scriptblock] $DownloadAdapter)
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $Layout.downloads
    [void] [IO.Directory]::CreateDirectory($Layout.downloads)
    $cachePath = Get-IrodoriArtifactCachePath -Layout $Layout -Artifact $Artifact
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $cachePath
    if (Test-IrodoriVerifiedFile -Path $cachePath -Artifact $Artifact) { return $cachePath }
    if (Test-Path -LiteralPath $cachePath) { Remove-IrodoriManagedItem -Layout $Layout -Path $cachePath }
    $partialPath = $cachePath + '.partial'
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $partialPath
    if (Test-Path -LiteralPath $partialPath) { Remove-IrodoriManagedItem -Layout $Layout -Path $partialPath }
    try {
        if (-not (Test-IrodoriHttpsUrl $Artifact.url)) { throw 'Irodori artifact URL must use HTTPS.' }
        $download = & $DownloadAdapter $Artifact $partialPath ([int64] $Artifact.size)
        if ($null -eq $download -or $download.PSObject.Properties.Name -notcontains 'final_url' -or -not (Test-IrodoriHttpsUrl ([string] $download.final_url))) { throw 'Irodori artifact final URL must use HTTPS.' }
        if ($download.PSObject.Properties.Name -contains 'cancelled' -and $download.cancelled) { throw [OperationCanceledException]::new('Irodori artifact download was cancelled.') }
        if (-not (Test-IrodoriVerifiedFile -Path $partialPath -Artifact $Artifact)) { throw 'Irodori artifact size or SHA-256 does not match the manifest.' }
        [IO.File]::Move($partialPath, $cachePath)
        return $cachePath
    } finally {
        if (Test-Path -LiteralPath $partialPath) { Remove-IrodoriManagedItem -Layout $Layout -Path $partialPath }
    }
}

function Get-IrodoriZipEntryPath {
    param([IO.Compression.ZipArchiveEntry] $Entry)
    $name = $Entry.FullName.Replace('\', '/')
    if ([string]::IsNullOrWhiteSpace($name) -or $name.Contains(':') -or $name.StartsWith('/') -or $name.StartsWith('\') -or [IO.Path]::IsPathRooted($name) -or $name -match '^[A-Za-z]:') { throw 'Irodori ZIP contains a rooted path or alternate data stream.' }
    $trimmed = $name.TrimEnd('/')
    $parts = @($trimmed -split '/')
    if ($parts.Count -eq 0 -or @($parts | Where-Object { $_ -in @('', '.', '..') }).Count -gt 0) { throw 'Irodori ZIP contains an unsafe relative path.' }
    $external = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int32] $Entry.ExternalAttributes), 0)
    $unixType = ($external -shr 16) -band 0xF000
    if ($unixType -eq 0xA000 -or ($unixType -notin @(0, 0x4000, 0x8000)) -or ($external -band 0x400) -ne 0) { throw 'Irodori ZIP contains a link or reparse point.' }
    return [pscustomobject]@{ entry = $Entry; path = $trimmed; is_directory = $name.EndsWith('/') -or $unixType -eq 0x4000 }
}

function Expand-IrodoriVerifiedZip {
    param([string] $ArchivePath, [string] $Destination, [switch] $StripSingleRoot)
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [void] [IO.Directory]::CreateDirectory($Destination)
    $archive = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $validated = [System.Collections.ArrayList]::new()
        $targets = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $value = Get-IrodoriZipEntryPath $entry
            if (-not $targets.Add($value.path)) { throw 'Irodori ZIP contains duplicate case-folded targets.' }
            [void] $validated.Add($value)
        }
        if ($validated.Count -eq 0) { throw 'Irodori ZIP is empty.' }
        foreach ($value in $validated) {
            $parts = @($value.path -split '/')
            for ($index = 1; $index -lt $parts.Count; $index++) {
                $ancestor = ($parts[0..($index - 1)] -join '/')
                $ancestorEntry = @($validated | Where-Object { $_.path -eq $ancestor })
                if ($ancestorEntry.Count -gt 0 -and -not $ancestorEntry[0].is_directory) { throw 'Irodori ZIP contains a file-directory collision.' }
            }
        }
        $stripRoot = $null
        if ($StripSingleRoot) {
            $roots = @($validated | ForEach-Object { ($_.path -split '/')[0] } | Sort-Object -Unique)
            if ($roots.Count -ne 1 -or @($validated | Where-Object { $_.path -notmatch '/' -and -not $_.is_directory }).Count -gt 0) { throw 'Irodori server ZIP must contain one top-level directory.' }
            $stripRoot = $roots[0] + '/'
        }
        foreach ($value in $validated) {
            $relative = if ($null -ne $stripRoot -and $value.path -eq $roots[0] -and $value.is_directory) { '' } elseif ($null -ne $stripRoot) { $value.path.Substring($stripRoot.Length) } else { $value.path }
            if ([string]::IsNullOrEmpty($relative)) { continue }
            $target = Join-Path $Destination ($relative.Replace('/', [IO.Path]::DirectorySeparatorChar))
            if (-not (Test-IrodoriDescendantPath $Destination $target)) { throw 'Irodori ZIP extraction target escapes its destination.' }
            if ($value.is_directory) {
                [void] [IO.Directory]::CreateDirectory($target)
                continue
            }
            [void] [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
            $input = $value.entry.Open()
            $output = [IO.FileStream]::new($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try { $input.CopyTo($output); $output.Flush($true) } finally { $output.Dispose(); $input.Dispose() }
        }
    } finally { $archive.Dispose() }
    foreach ($item in @(Get-ChildItem -LiteralPath $Destination -Force -Recurse)) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Irodori ZIP extraction produced a reparse point.' }
    }
}

function Get-IrodoriVerifiedZipInventory {
    param([string] $ArchivePath, [psobject] $Artifact, [switch] $StripSingleRoot)
    if (-not (Test-IrodoriVerifiedFile -Path $ArchivePath -Artifact $Artifact)) { throw 'Irodori cached ZIP does not match the manifest.' }
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $validated = [System.Collections.ArrayList]::new()
        $targets = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $value = Get-IrodoriZipEntryPath $entry
            if (-not $targets.Add($value.path)) { throw 'Irodori ZIP contains duplicate case-folded targets.' }
            [void] $validated.Add($value)
        }
        if ($validated.Count -eq 0) { throw 'Irodori ZIP is empty.' }
        foreach ($value in $validated) {
            $parts = @($value.path -split '/')
            for ($index = 1; $index -lt $parts.Count; $index++) {
                $ancestor = ($parts[0..($index - 1)] -join '/')
                $ancestorEntry = @($validated | Where-Object { $_.path -eq $ancestor })
                if ($ancestorEntry.Count -gt 0 -and -not $ancestorEntry[0].is_directory) { throw 'Irodori ZIP contains a file-directory collision.' }
            }
        }
        $stripRoot = $null
        $rootName = $null
        if ($StripSingleRoot) {
            $roots = @($validated | ForEach-Object { ($_.path -split '/')[0] } | Sort-Object -Unique)
            if ($roots.Count -ne 1 -or @($validated | Where-Object { $_.path -notmatch '/' -and -not $_.is_directory }).Count -gt 0) { throw 'Irodori server ZIP must contain one top-level directory.' }
            $rootName = $roots[0]
            $stripRoot = $rootName + '/'
        }
        $inventory = [System.Collections.ArrayList]::new()
        foreach ($value in $validated) {
            $relative = if ($null -ne $stripRoot -and $value.path -eq $rootName -and $value.is_directory) { '' } elseif ($null -ne $stripRoot) { $value.path.Substring($stripRoot.Length) } else { $value.path }
            if ([string]::IsNullOrEmpty($relative) -or $value.is_directory) { continue }
            $input = $value.entry.Open()
            $sha256 = [Security.Cryptography.SHA256]::Create()
            try { $hash = [BitConverter]::ToString($sha256.ComputeHash($input)).Replace('-', '').ToLowerInvariant() } finally { $sha256.Dispose(); $input.Dispose() }
            [void] $inventory.Add([pscustomobject]@{ path = $relative; size = [int64] $value.entry.Length; sha256 = $hash })
        }
        return @($inventory)
    } finally { $archive.Dispose() }
}

function Test-IrodoriInstalledZipTree {
    param([string] $ArchivePath, [psobject] $Artifact, [string] $Destination, [switch] $StripSingleRoot)
    if (-not (Test-Path -LiteralPath $Destination -PathType Container)) { return $false }
    $expectedItems = @(Get-IrodoriVerifiedZipInventory -ArchivePath $ArchivePath -Artifact $Artifact -StripSingleRoot:$StripSingleRoot)
    $expected = [System.Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($item in $expectedItems) {
        if ($expected.ContainsKey([string] $item.path)) { return $false }
        $expected.Add([string] $item.path, $item)
    }
    $actual = [System.Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($item in @(Get-ChildItem -LiteralPath $Destination -Force -Recurse)) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { return $false }
        if ($item.PSIsContainer) { continue }
        $relative = $item.FullName.Substring((Get-IrodoriCanonicalPath $Destination).Length).TrimStart([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar).Replace('\', '/')
        if ($actual.ContainsKey($relative)) { return $false }
        $actual.Add($relative, $item)
    }
    if ($actual.Count -ne $expected.Count) { return $false }
    foreach ($pair in $expected.GetEnumerator()) {
        if (-not $actual.ContainsKey($pair.Key)) { return $false }
        $actualItem = $actual[$pair.Key]
        if ($actualItem.Length -ne $pair.Value.size -or (Get-FileHash -LiteralPath $actualItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $pair.Value.sha256) { return $false }
    }
    return $true
}

function Assert-IrodoriTransactionPaths {
    param([hashtable] $Layout, [psobject] $Transaction, [string] $TransactionPath)
    foreach ($field in @('schema_version', 'manifest_version', 'backend', 'phase', 'staging_path', 'runtime_path', 'backup_path')) {
        if ($Transaction.PSObject.Properties.Name -notcontains $field) { throw 'Irodori transaction is missing required fields.' }
    }
    $expectedName = $Transaction.manifest_version + '.json'
    if ($Transaction.schema_version -ne 1 -or $Transaction.manifest_version -ne [IO.Path]::GetFileName($Layout.runtime) -or $Transaction.backend -notin $AllowedBackends -or [IO.Path]::GetFileName($TransactionPath) -ne $expectedName -or $Transaction.phase -notin @('building', 'staged', 'promoting', 'constructing', 'publishing', 'committing', 'complete') -or (Get-IrodoriCanonicalPath $Transaction.runtime_path) -ne (Get-IrodoriCanonicalPath $Layout.runtime)) { throw 'Irodori transaction does not match the runtime.' }
    foreach ($pair in @(@($Transaction.staging_path, '.staging-'), @($Transaction.backup_path, '.backup-'))) {
        $candidate = Get-IrodoriCanonicalPath $pair[0]
        if ((Get-IrodoriCanonicalPath ([IO.Path]::GetDirectoryName($candidate))) -ne (Get-IrodoriCanonicalPath $Layout.runtime_root) -or -not [IO.Path]::GetFileName($candidate).StartsWith($pair[1], [StringComparison]::Ordinal)) { throw 'Irodori transaction contains an unsafe cleanup path.' }
        Assert-IrodoriCleanupTarget -Root $Layout.root -Target $candidate
    }
}

function Recover-IrodoriTransaction {
    param([hashtable] $Layout, [string] $TransactionPath)
    if (-not (Test-Path -LiteralPath $TransactionPath -PathType Leaf)) { return }
    try { $transaction = Get-Content -LiteralPath $TransactionPath -Raw | ConvertFrom-Json -ErrorAction Stop } catch { throw 'Irodori transaction is not valid JSON.' }
    Assert-IrodoriTransactionPaths -Layout $Layout -Transaction $transaction -TransactionPath $TransactionPath
    if ($transaction.phase -eq 'promoting') {
        $hasStaging = Test-Path -LiteralPath $transaction.staging_path
        $hasRuntime = Test-Path -LiteralPath $transaction.runtime_path
        $hasBackup = Test-Path -LiteralPath $transaction.backup_path
        if ($hasRuntime -and $hasBackup) {
            Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.runtime_path
            Move-Item -LiteralPath $transaction.backup_path -Destination $transaction.runtime_path
        } elseif (-not $hasRuntime -and $hasBackup) {
            Move-Item -LiteralPath $transaction.backup_path -Destination $transaction.runtime_path
        } elseif ($hasRuntime -and -not $hasStaging) {
            Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.runtime_path
        }
    } elseif ($transaction.phase -in @('constructing', 'publishing')) {
        if (Test-Path -LiteralPath $transaction.runtime_path) { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.runtime_path }
        if (Test-Path -LiteralPath $transaction.backup_path) { Move-Item -LiteralPath $transaction.backup_path -Destination $transaction.runtime_path }
    } elseif ($transaction.phase -eq 'committing') {
        $activeMatches = $false
        if (Test-Path -LiteralPath $Layout.active_marker -PathType Leaf) {
            try {
                $active = Get-Content -LiteralPath $Layout.active_marker -Raw | ConvertFrom-Json -ErrorAction Stop
                $activeMatches = $active.manifest_version -eq $transaction.manifest_version -and $active.backend -eq $transaction.backend -and (Get-IrodoriCanonicalPath $active.runtime_path) -eq (Get-IrodoriCanonicalPath $transaction.runtime_path)
            } catch { $activeMatches = $false }
        }
        if ($activeMatches) {
            if (Test-Path -LiteralPath $transaction.backup_path) { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.backup_path }
        } else {
            if (Test-Path -LiteralPath $transaction.runtime_path) { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.runtime_path }
            if (Test-Path -LiteralPath $transaction.backup_path) { Move-Item -LiteralPath $transaction.backup_path -Destination $transaction.runtime_path }
        }
    } elseif ($transaction.phase -eq 'complete') {
        if (Test-Path -LiteralPath $transaction.backup_path) { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.backup_path }
    }
    if (Test-Path -LiteralPath $transaction.staging_path) { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.staging_path }
    if (Test-Path -LiteralPath $transaction.backup_path) {
        if (-not (Test-Path -LiteralPath $transaction.runtime_path)) { Move-Item -LiteralPath $transaction.backup_path -Destination $transaction.runtime_path }
        else { Remove-IrodoriManagedItem -Layout $Layout -Path $transaction.backup_path }
    }
    Remove-IrodoriManagedItem -Layout $Layout -Path $TransactionPath
}

function Invoke-IrodoriDefaultRunApp {
    param([string] $Executable, [string[]] $Arguments, [string] $WorkingDirectory, [hashtable] $Environment)
    $previous = @{}
    foreach ($key in $Environment.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, [EnvironmentVariableTarget]::Process)
        [Environment]::SetEnvironmentVariable($key, [string] $Environment[$key], [EnvironmentVariableTarget]::Process)
    }
    Push-Location -LiteralPath $WorkingDirectory
    try {
        $output = & $Executable @Arguments 2>&1
        $exitCode = $LASTEXITCODE
        return [pscustomobject]@{ exit_code = $exitCode; cancelled = $false; output = @($output | ForEach-Object { [string] $_ }) }
    } finally {
        Pop-Location
        foreach ($key in $Environment.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key], [EnvironmentVariableTarget]::Process) }
    }
}

function Assert-IrodoriRunSucceeded {
    param([object] $Result, [string] $Stage)
    if ($null -eq $Result) { throw "Irodori $Stage returned no result." }
    if ($Result.PSObject.Properties.Name -contains 'cancelled' -and $Result.cancelled) { throw [OperationCanceledException]::new("Irodori $Stage was cancelled.") }
    if ($Result.PSObject.Properties.Name -notcontains 'exit_code' -or [int] $Result.exit_code -ne 0) { throw "Irodori $Stage failed." }
}

function Get-IrodoriArtifactById {
    param([psobject] $Manifest, [string] $Id)
    $matches = @($Manifest.artifacts | Where-Object { $_.id -eq $Id })
    if ($matches.Count -ne 1) { throw "Irodori manifest must contain artifact: $Id" }
    return $matches[0]
}

function Get-IrodoriRuntimeEnvironment {
    param([hashtable] $Layout)
    return @{
        UV_PYTHON_INSTALL_DIR = Join-Path $Layout.runtime 'python'
        UV_PROJECT_ENVIRONMENT = Join-Path $Layout.runtime 'env'
        UV_CACHE_DIR = Join-Path $Layout.runtime 'cache\uv'
        HF_HOME = Join-Path $Layout.runtime 'hf'
        UV_NO_SYSTEM_CONFIG = '1'
        HF_HUB_OFFLINE = '1'
        TRANSFORMERS_OFFLINE = '1'
        PYTHONDONTWRITEBYTECODE = '1'
    }
}

function Test-IrodoriReusableRuntime {
    param(
        [hashtable] $Layout,
        [psobject] $Manifest,
        [string] $Backend,
        [psobject] $UvArtifact,
        [psobject] $ServerArtifact,
        [psobject[]] $VerifiedArtifacts,
        [psobject[]] $TokenizerArtifacts,
        [scriptblock] $RunAdapter
    )
    if (-not (Test-IrodoriCompletion -Layout $Layout -Manifest $Manifest -ExpectedBackend $Backend)) { return $false }
    try {
        $uvPath = Join-Path $Layout.runtime $UvArtifact.install_relative_path
        $serverPath = Join-Path $Layout.runtime $ServerArtifact.install_relative_path
        Assert-IrodoriCleanupTarget -Root $Layout.root -Target $uvPath
        Assert-IrodoriCleanupTarget -Root $Layout.root -Target $serverPath
        $uvArchive = Get-IrodoriArtifactCachePath -Layout $Layout -Artifact $UvArtifact
        $serverArchive = Get-IrodoriArtifactCachePath -Layout $Layout -Artifact $ServerArtifact
        $uvTree = Join-Path $Layout.runtime ([IO.Path]::GetDirectoryName($UvArtifact.install_relative_path))
        if (-not (Test-IrodoriInstalledZipTree -ArchivePath $uvArchive -Artifact $UvArtifact -Destination $uvTree) -or -not (Test-IrodoriInstalledZipTree -ArchivePath $serverArchive -Artifact $ServerArtifact -Destination $serverPath -StripSingleRoot)) { return $false }
        foreach ($artifact in $VerifiedArtifacts) {
            if (-not (Test-IrodoriVerifiedFile -Path (Join-Path $Layout.runtime $artifact.install_relative_path) -Artifact $artifact)) { return $false }
        }
        $revision = '5fb086c49f49824cfc93f09cc4ed5cd5917bef3d'
        $snapshot = Join-Path $Layout.runtime ('hf\hub\models--sbintuitions--sarashina2.2-0.5b\snapshots\' + $revision)
        $snapshotNames = @('tokenizer.model', 'tokenizer_config.json', 'config.json')
        for ($index = 0; $index -lt $TokenizerArtifacts.Count; $index++) {
            if (-not (Test-IrodoriVerifiedFile -Path (Join-Path $snapshot $snapshotNames[$index]) -Artifact $TokenizerArtifacts[$index])) { return $false }
        }
        $referencePath = Join-Path $Layout.runtime 'hf\hub\models--sbintuitions--sarashina2.2-0.5b\refs\main'
        if (-not (Test-Path -LiteralPath $referencePath -PathType Leaf) -or (Get-Content -LiteralPath $referencePath -Raw).Trim() -ne $revision) { return $false }
        $verification = & $RunAdapter $uvPath @('run', '--no-sync', 'python', '-c', 'import irodori_openai_tts') $serverPath (Get-IrodoriRuntimeEnvironment -Layout $Layout)
        Assert-IrodoriRunSucceeded -Result $verification -Stage 'reuse environment verification'
        return $true
    } catch { return $false }
}

function Get-IrodoriProvisionMutexName {
    param([string] $Root, [string] $ManifestVersion)
    $material = (Get-IrodoriCanonicalPath $Root).ToLowerInvariant() + '|' + $ManifestVersion
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($material))).Replace('-', '').ToLowerInvariant() } finally { $sha256.Dispose() }
    return 'Local\ParallelWorld.Irodori.Provision.' + $hash
}

function Enter-IrodoriProvisionMutex {
    param([string] $Name, [int] $TimeoutMilliseconds)
    $mutex = [Threading.Mutex]::new($false, $Name)
    $acquired = $false
    try {
        try { $acquired = $mutex.WaitOne($TimeoutMilliseconds) } catch [Threading.AbandonedMutexException] { $acquired = $true }
        if (-not $acquired) { throw 'Irodori provisioning lock timeout.' }
        return $mutex
    } catch {
        $mutex.Dispose()
        throw
    }
}

function Invoke-IrodoriProvisionLocked {
    param(
        [psobject] $Manifest,
        [hashtable] $Layout,
        [Parameter(Mandatory)] [ValidateSet('cpu', 'cu128')] [string] $Backend,
        [hashtable] $Adapters = @{}
    )
    if ($Manifest.manifest_version -notmatch '^\d{4}-\d{2}-\d{2}\.\d+$' -or $Backend -notin @($Manifest.backends)) { throw 'Irodori provisioning inputs are invalid.' }
    if (-not $Layout.ContainsKey('root')) { throw 'Irodori layout is missing: root' }
    $expectedLayout = Get-IrodoriLayout -Root $Layout.root -ManifestVersion $Manifest.manifest_version
    foreach ($key in $expectedLayout.Keys) {
        if (-not $Layout.ContainsKey($key) -or (Get-IrodoriCanonicalPath $Layout[$key]) -ne (Get-IrodoriCanonicalPath $expectedLayout[$key])) { throw "Irodori layout path is missing or invalid: $key" }
    }
    Assert-IrodoriCleanupTarget -Root $Layout.root -Target $Layout.runtime
    foreach ($managedPath in @($Layout.cache_root, $Layout.downloads, $Layout.transactions)) {
        Assert-IrodoriCleanupTarget -Root $Layout.root -Target $managedPath
    }
    $uvArtifact = Get-IrodoriArtifactById $Manifest 'uv-windows-x86_64'
    $serverArtifact = Get-IrodoriArtifactById $Manifest 'irodori-server'
    $modelArtifact = Get-IrodoriArtifactById $Manifest 'irodori-model'
    $codecArtifact = Get-IrodoriArtifactById $Manifest 'irodori-codec'
    $tokenizerModel = Get-IrodoriArtifactById $Manifest 'sarashina-tokenizer-model'
    $tokenizerConfig = Get-IrodoriArtifactById $Manifest 'sarashina-tokenizer-config'
    $modelConfig = Get-IrodoriArtifactById $Manifest 'sarashina-config'

    $runAdapter = if ($Adapters.ContainsKey('RunApp')) { $Adapters.RunApp } else {
        { param($Executable, $Arguments, $WorkingDirectory, $Environment) Invoke-IrodoriDefaultRunApp -Executable $Executable -Arguments $Arguments -WorkingDirectory $WorkingDirectory -Environment $Environment }
    }
    [void] [IO.Directory]::CreateDirectory($Layout.root)
    [void] [IO.Directory]::CreateDirectory($Layout.runtime_root)
    [void] [IO.Directory]::CreateDirectory($Layout.transactions)
    $transactionPath = Join-Path $Layout.transactions ($Manifest.manifest_version + '.json')
    Recover-IrodoriTransaction -Layout $Layout -TransactionPath $transactionPath
    $verifiedArtifacts = @($modelArtifact, $codecArtifact, $tokenizerModel, $tokenizerConfig, $modelConfig)
    $tokenizerArtifacts = @($tokenizerModel, $tokenizerConfig, $modelConfig)
    if (Test-IrodoriReusableRuntime -Layout $Layout -Manifest $Manifest -Backend $Backend -UvArtifact $uvArtifact -ServerArtifact $serverArtifact -VerifiedArtifacts $verifiedArtifacts -TokenizerArtifacts $tokenizerArtifacts -RunAdapter $runAdapter) {
        return [pscustomobject]@{ status = 'reused'; runtime_path = $Layout.runtime; uv_path = Join-Path $Layout.runtime $uvArtifact.install_relative_path }
    }

    [int64] $artifactBytes = 0
    foreach ($artifact in @($Manifest.artifacts)) {
        $size = [int64] $artifact.size
        if ($artifactBytes -gt [int64]::MaxValue - $size) { throw 'Irodori manifest artifact sizes overflow the disk estimate.' }
        $artifactBytes += $size
    }
    if ($artifactBytes -gt ([int64]::MaxValue - 2147483648) / 2) { throw 'Irodori manifest artifact sizes overflow the disk estimate.' }
    [int64] $requiredBytes = ($artifactBytes * 2) + 2147483648
    $getFreeBytes = if ($Adapters.ContainsKey('GetFreeBytes')) { $Adapters.GetFreeBytes } else {
        { param($Path) return [IO.DriveInfo]::new([IO.Path]::GetPathRoot([IO.Path]::GetFullPath($Path))).AvailableFreeSpace }
    }
    if ([int64] (& $getFreeBytes $Layout.root) -lt $requiredBytes) { throw 'Irodori provisioning does not have enough free disk space.' }
    $downloadAdapter = if ($Adapters.ContainsKey('DownloadArtifact')) { $Adapters.DownloadArtifact } else {
        { param($Artifact, $PartialPath, $MaximumBytes) Invoke-IrodoriHttpDownload -Artifact $Artifact -PartialPath $PartialPath -MaximumBytes $MaximumBytes }
    }
    $writeProgress = if ($Adapters.ContainsKey('WriteProgress')) { $Adapters.WriteProgress } else { { param($Stage, $Message) Write-Verbose "[$Stage] $Message" } }

    $nonce = [Guid]::NewGuid().ToString('N')
    $stagingPath = Join-Path $Layout.runtime_root ('.staging-' + $Manifest.manifest_version + '-' + $Backend + '-' + $nonce)
    $backupPath = Join-Path $Layout.runtime_root ('.backup-' + $Manifest.manifest_version + '-' + $Backend)
    $transaction = [ordered]@{
        schema_version = 1; manifest_version = $Manifest.manifest_version; backend = $Backend; phase = 'building'
        staging_path = $stagingPath; runtime_path = $Layout.runtime; backup_path = $backupPath
    }
    Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
    try {
        Assert-IrodoriCleanupTarget -Root $Layout.root -Target $stagingPath
        [void] [IO.Directory]::CreateDirectory($stagingPath)
        $cached = @{}
        foreach ($artifact in @($Manifest.artifacts)) {
            & $writeProgress 'download' $artifact.id
            $cached[$artifact.id] = Get-IrodoriVerifiedArtifact -Layout $Layout -Artifact $artifact -DownloadAdapter $downloadAdapter
        }
        $uvDestination = Join-Path $stagingPath ([IO.Path]::GetDirectoryName($uvArtifact.install_relative_path))
        Expand-IrodoriVerifiedZip -ArchivePath $cached[$uvArtifact.id] -Destination $uvDestination
        $serverDestination = Join-Path $stagingPath $serverArtifact.install_relative_path
        Expand-IrodoriVerifiedZip -ArchivePath $cached[$serverArtifact.id] -Destination $serverDestination -StripSingleRoot
        foreach ($artifact in @($modelArtifact, $codecArtifact, $tokenizerModel, $tokenizerConfig, $modelConfig)) {
            $destination = Join-Path $stagingPath $artifact.install_relative_path
            if (-not (Test-IrodoriDescendantPath $stagingPath $destination)) { throw 'Irodori artifact install path escapes staging.' }
            Assert-IrodoriCleanupTarget -Root $Layout.root -Target $destination
            [void] [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
            [IO.File]::Copy($cached[$artifact.id], $destination, $false)
        }
        $revision = '5fb086c49f49824cfc93f09cc4ed5cd5917bef3d'
        $hfRepository = Join-Path $stagingPath 'hf\hub\models--sbintuitions--sarashina2.2-0.5b'
        $snapshot = Join-Path $hfRepository ('snapshots\' + $revision)
        [void] [IO.Directory]::CreateDirectory($snapshot)
        [IO.File]::Copy((Join-Path $stagingPath $tokenizerModel.install_relative_path), (Join-Path $snapshot 'tokenizer.model'), $false)
        [IO.File]::Copy((Join-Path $stagingPath $tokenizerConfig.install_relative_path), (Join-Path $snapshot 'tokenizer_config.json'), $false)
        [IO.File]::Copy((Join-Path $stagingPath $modelConfig.install_relative_path), (Join-Path $snapshot 'config.json'), $false)
        Write-IrodoriAtomicText -Layout $Layout -Path (Join-Path $hfRepository 'refs\main') -Text $revision

        $transaction.phase = 'staged'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
        $transaction.phase = 'promoting'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
        if (Test-Path -LiteralPath $backupPath) { Remove-IrodoriManagedItem -Layout $Layout -Path $backupPath }
        if (Test-Path -LiteralPath $Layout.runtime) { Move-Item -LiteralPath $Layout.runtime -Destination $backupPath }
        Move-Item -LiteralPath $stagingPath -Destination $Layout.runtime
        $transaction.phase = 'constructing'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction

        $uvPath = Join-Path $Layout.runtime $uvArtifact.install_relative_path
        $serverPath = Join-Path $Layout.runtime $serverArtifact.install_relative_path
        if (-not (Test-Path -LiteralPath $uvPath -PathType Leaf) -or -not (Test-IrodoriDescendantPath $Layout.runtime $uvPath)) { throw 'Verified managed uv.exe is missing.' }
        $environment = Get-IrodoriRuntimeEnvironment -Layout $Layout
        $pythonInstall = & $runAdapter $uvPath @('python', 'install', $Manifest.python_version) $serverPath $environment
        Assert-IrodoriRunSucceeded -Result $pythonInstall -Stage 'Python installation'
        $sync = & $runAdapter $uvPath @('sync', '--frozen', '--extra', $Backend, '--python', $Manifest.python_version, '--managed-python') $serverPath $environment
        Assert-IrodoriRunSucceeded -Result $sync -Stage 'environment sync'
        foreach ($artifact in @($modelArtifact, $codecArtifact, $tokenizerModel, $tokenizerConfig, $modelConfig)) {
            if (-not (Test-IrodoriVerifiedFile -Path (Join-Path $Layout.runtime $artifact.install_relative_path) -Artifact $artifact)) { throw "Irodori installed artifact verification failed: $($artifact.id)" }
        }
        foreach ($pair in @(@($tokenizerModel, 'tokenizer.model'), @($tokenizerConfig, 'tokenizer_config.json'), @($modelConfig, 'config.json'))) {
            if (-not (Test-IrodoriVerifiedFile -Path (Join-Path (Join-Path $Layout.runtime ('hf\hub\models--sbintuitions--sarashina2.2-0.5b\snapshots\' + $revision)) $pair[1]) -Artifact $pair[0])) { throw 'Irodori tokenizer cache verification failed.' }
        }
        $verification = & $runAdapter $uvPath @('run', '--no-sync', 'python', '-c', 'import irodori_openai_tts') $serverPath $environment
        Assert-IrodoriRunSucceeded -Result $verification -Stage 'environment verification'

        $transaction.phase = 'publishing'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
        $completedAt = [DateTimeOffset]::UtcNow.ToString('o')
        Write-IrodoriJson -Layout $Layout -Path $Layout.completion_marker -Value ([ordered]@{
            schema_version = 1; manifest_version = $Manifest.manifest_version; backend = $Backend
            python_version = $Manifest.python_version; completed_at = $completedAt
        })
        if (-not (Test-IrodoriCompletion -Layout $Layout -Manifest $Manifest -ExpectedBackend $Backend)) { throw 'Irodori completion verification failed.' }
        $transaction.phase = 'committing'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
        Write-IrodoriJson -Layout $Layout -Path $Layout.active_marker -Value ([ordered]@{
            schema_version = 1; manifest_version = $Manifest.manifest_version; backend = $Backend
            runtime_path = $Layout.runtime; completed_at = $completedAt
        })
        $transaction.phase = 'complete'
        Write-IrodoriJson -Layout $Layout -Path $transactionPath -Value $transaction
        if (Test-Path -LiteralPath $backupPath) { Remove-IrodoriManagedItem -Layout $Layout -Path $backupPath }
        Remove-IrodoriManagedItem -Layout $Layout -Path $transactionPath
        return [pscustomobject]@{ status = 'provisioned'; runtime_path = $Layout.runtime; uv_path = $uvPath }
    } catch {
        try { Recover-IrodoriTransaction -Layout $Layout -TransactionPath $transactionPath } catch { }
        throw
    }
}

function Invoke-IrodoriProvision {
    param(
        [psobject] $Manifest,
        [hashtable] $Layout,
        [Parameter(Mandatory)] [ValidateSet('cpu', 'cu128')] [string] $Backend,
        [hashtable] $Adapters = @{}
    )
    if (-not $Layout.ContainsKey('root') -or $Manifest.manifest_version -notmatch '^\d{4}-\d{2}-\d{2}\.\d+$' -or $Backend -notin @($Manifest.backends)) { throw 'Irodori provisioning lock inputs are invalid.' }
    $timeoutMilliseconds = if ($Adapters.ContainsKey('LockTimeoutMilliseconds')) { [int] $Adapters.LockTimeoutMilliseconds } else { 30000 }
    if ($timeoutMilliseconds -lt 0 -or $timeoutMilliseconds -gt 600000) { throw 'Irodori provisioning lock timeout must be between 0 and 600000 milliseconds.' }
    $mutexName = Get-IrodoriProvisionMutexName -Root $Layout.root -ManifestVersion $Manifest.manifest_version
    $mutex = Enter-IrodoriProvisionMutex -Name $mutexName -TimeoutMilliseconds $timeoutMilliseconds
    try {
        return Invoke-IrodoriProvisionLocked -Manifest $Manifest -Layout $Layout -Backend $Backend -Adapters $Adapters
    } finally {
        try { $mutex.ReleaseMutex() } finally { $mutex.Dispose() }
    }
}

function Invoke-IrodoriBootstrapHttp {
    param([string] $Method, [string] $Uri, [object] $Body)
    if ($Method -eq 'GET') {
        $response = Invoke-RestMethod -Uri $Uri -Method Get -TimeoutSec 10
        return [pscustomobject]@{ status_code = 200; body = $response; bytes = $null }
    }
    $json = $Body | ConvertTo-Json -Depth 5 -Compress
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ('parallel-world-irodori-warmup-' + [Guid]::NewGuid().ToString('N') + '.wav')
    try {
        $response = Invoke-WebRequest -Uri $Uri -Method Post -ContentType 'application/json' -Body $json -OutFile $temporary -TimeoutSec 300 -UseBasicParsing
        return [pscustomobject]@{ status_code = [int] $response.StatusCode; body = $null; bytes = [IO.File]::ReadAllBytes($temporary) }
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Test-IrodoriBootstrapHealth {
    param([scriptblock] $InvokeHttp, [int] $Port)
    try {
        $response = & $InvokeHttp 'GET' "http://127.0.0.1:$Port/health" $null
        return $null -ne $response -and [int] $response.status_code -eq 200
    } catch { return $false }
}

function Test-IrodoriWaveBytes {
    param([byte[]] $Bytes)
    return $null -ne $Bytes -and $Bytes.Length -ge 12 -and
        [Text.Encoding]::ASCII.GetString($Bytes, 0, 4) -eq 'RIFF' -and
        [Text.Encoding]::ASCII.GetString($Bytes, 8, 4) -eq 'WAVE'
}

function Test-IrodoriRuntimeReady {
    param(
        [psobject] $Manifest,
        [hashtable] $Layout,
        [Parameter(Mandatory)] [ValidateSet('cpu', 'cu128')] [string] $Backend,
        [hashtable] $Adapters = @{}
    )
    if (-not (Test-IrodoriCompletion -Layout $Layout -Manifest $Manifest -ExpectedBackend $Backend)) { return $false }
    $transactionPath = Join-Path $Layout.transactions ($Manifest.manifest_version + '.json')
    if (Test-Path -LiteralPath $transactionPath) { return $false }
    $uvArtifact = Get-IrodoriArtifactById $Manifest 'uv-windows-x86_64'
    $serverArtifact = Get-IrodoriArtifactById $Manifest 'irodori-server'
    $modelArtifact = Get-IrodoriArtifactById $Manifest 'irodori-model'
    $codecArtifact = Get-IrodoriArtifactById $Manifest 'irodori-codec'
    $tokenizerModel = Get-IrodoriArtifactById $Manifest 'sarashina-tokenizer-model'
    $tokenizerConfig = Get-IrodoriArtifactById $Manifest 'sarashina-tokenizer-config'
    $modelConfig = Get-IrodoriArtifactById $Manifest 'sarashina-config'
    $runAdapter = if ($Adapters.ContainsKey('RunCommand')) { $Adapters.RunCommand } else {
        { param($Executable, $Arguments, $WorkingDirectory, $Environment) Invoke-IrodoriDefaultRunApp -Executable $Executable -Arguments $Arguments -WorkingDirectory $WorkingDirectory -Environment $Environment }
    }
    return Test-IrodoriReusableRuntime -Layout $Layout -Manifest $Manifest -Backend $Backend `
        -UvArtifact $uvArtifact -ServerArtifact $serverArtifact `
        -VerifiedArtifacts @($modelArtifact, $codecArtifact, $tokenizerModel, $tokenizerConfig, $modelConfig) `
        -TokenizerArtifacts @($tokenizerModel, $tokenizerConfig, $modelConfig) -RunAdapter $runAdapter
}

function Invoke-IrodoriBootstrap {
    param(
        [Parameter(Mandatory)] [string] $ManifestPath,
        [Parameter(Mandatory)] [string] $DataRoot,
        [hashtable] $Adapters = @{}
    )
    $runApp = if ($Adapters.ContainsKey('RunApp')) { $Adapters.RunApp } else {
        { & (Join-Path $PSScriptRoot 'dev-up.ps1'); return $LASTEXITCODE }
    }
    $explicitEngine = [Environment]::GetEnvironmentVariable('PW_TTS_ENGINE')
    if (-not [string]::IsNullOrWhiteSpace($explicitEngine) -and $explicitEngine.ToLowerInvariant() -ne 'irodori') {
        $appExitCode = & $runApp
        return [pscustomobject]@{ status = 'external_engine'; app_exit_code = [int] $appExitCode; backend = $null }
    }

    $status = 'setup_failed'
    $backend = $null
    $owned = $null
    $savedEnvironment = @{}
    $managedEnvironmentNames = @(
        'PATH', 'PW_TTS_ENGINE', 'PW_TTS_PORT', 'PW_IRODORI_DIR', 'IRODORI_CHECKPOINT',
        'IRODORI_CODEC_REPO', 'IRODORI_VOICES_DIR', 'IRODORI_COMPILE_MODEL',
        'HF_HOME', 'HF_HUB_OFFLINE', 'TRANSFORMERS_OFFLINE'
    )
    foreach ($name in $managedEnvironmentNames) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name) }
    if ([string]::IsNullOrWhiteSpace($explicitEngine)) { $env:PW_TTS_ENGINE = 'irodori' }
    try {
        try {
            $manifest = Import-IrodoriManifest -Path $ManifestPath
            $layout = Get-IrodoriLayout -Root $DataRoot -ManifestVersion $manifest.manifest_version
            $detectGpuNames = if ($Adapters.ContainsKey('DetectGpuNames')) { $Adapters.DetectGpuNames } else {
                { @((Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })) }
            }
            $backend = Get-IrodoriBackend -GpuNames @(& $detectGpuNames)
            $testRuntime = if ($Adapters.ContainsKey('TestRuntime')) { $Adapters.TestRuntime } else {
                { param($Manifest, $Layout, $Backend, $RuntimeAdapters) Test-IrodoriRuntimeReady -Manifest $Manifest -Layout $Layout -Backend $Backend -Adapters $RuntimeAdapters }
            }
            $isComplete = [bool] (& $testRuntime $manifest $layout $backend $Adapters)
            if (-not $isComplete) {
                [int64] $downloadBytes = 0
                foreach ($artifact in @($manifest.artifacts)) { $downloadBytes += [int64] $artifact.size }
                [int64] $peakBytes = ($downloadBytes * 2) + 2147483648
                $promptMessage = @"
Irodori-TTS managed environment is not ready.
Backend: $backend
Direct downloads: $downloadBytes bytes
Conservative peak free space: $peakBytes bytes
Storage (LocalAppData): $DataRoot
Licenses: Irodori/model/codec/tokenizer MIT; uv Apache-2.0 OR MIT
Voice cloning / third-party voice: use only recordings with the speaker's explicit consent.
Build now? [Y/N]
"@
                $promptConsent = if ($Adapters.ContainsKey('PromptConsent')) { $Adapters.PromptConsent } else {
                    { param($Message) Write-Host $Message; return (Read-Host).Trim() -match '^(?i:y|yes)$' }
                }
                if (-not (& $promptConsent $promptMessage)) {
                    $status = 'declined'
                } else {
                    $provision = if ($Adapters.ContainsKey('Provision')) { $Adapters.Provision } else {
                        { param($Manifest, $Layout, $Backend, $ProvisionAdapters) Invoke-IrodoriProvision -Manifest $Manifest -Layout $Layout -Backend $Backend -Adapters $ProvisionAdapters }
                    }
                    $provisionResult = & $provision $manifest $layout $backend $Adapters
                    $isComplete = $true
                }
            } else {
                $provision = if ($Adapters.ContainsKey('Provision')) { $Adapters.Provision } else {
                    { param($Manifest, $Layout, $Backend, $ProvisionAdapters) Invoke-IrodoriProvision -Manifest $Manifest -Layout $Layout -Backend $Backend -Adapters $ProvisionAdapters }
                }
                $provisionResult = & $provision $manifest $layout $backend $Adapters
            }

            if ($isComplete) {
                $uvPath = [string] $provisionResult.uv_path
                $serverPath = Join-Path $layout.runtime 'server'
                $runtimeEnvironment = [ordered]@{
                    IRODORI_CHECKPOINT = Join-Path $layout.runtime 'models\model.safetensors'
                    IRODORI_CODEC_REPO = Join-Path $layout.runtime 'models\codec\weights.pth'
                    IRODORI_VOICES_DIR = $layout.voices
                    IRODORI_COMPILE_MODEL = 'false'
                    HF_HOME = Join-Path $layout.runtime 'hf'
                    HF_HUB_OFFLINE = '1'
                    TRANSFORMERS_OFFLINE = '1'
                }
                [void] [IO.Directory]::CreateDirectory($layout.voices)
                [void] [IO.Directory]::CreateDirectory($layout.loras)
                $env:PATH = ([IO.Path]::GetDirectoryName($uvPath)) + [IO.Path]::PathSeparator + $env:PATH
                $env:PW_TTS_ENGINE = 'irodori'
                $env:PW_TTS_PORT = '8088'
                $env:PW_IRODORI_DIR = $serverPath
                foreach ($entry in $runtimeEnvironment.GetEnumerator()) { [Environment]::SetEnvironmentVariable($entry.Key, [string] $entry.Value) }

                $invokeHttp = if ($Adapters.ContainsKey('InvokeHttp')) { $Adapters.InvokeHttp } else {
                    { param($Method, $Uri, $Body) Invoke-IrodoriBootstrapHttp -Method $Method -Uri $Uri -Body $Body }
                }
                $testPort = if ($Adapters.ContainsKey('TestPort')) { $Adapters.TestPort } else {
                    { param($Port) $client = [Net.Sockets.TcpClient]::new(); try { return $client.ConnectAsync('127.0.0.1', $Port).Wait(1000) -and $client.Connected } catch { return $false } finally { $client.Dispose() } }
                }
                $portWasOpen = [bool] (& $testPort 8088)
                if ($portWasOpen -and -not (Test-IrodoriBootstrapHealth -InvokeHttp $invokeHttp -Port 8088)) {
                    $status = 'port_conflict'
                } else {
                    if (-not $portWasOpen) {
                        $startOwned = if ($Adapters.ContainsKey('StartOwnedProcess')) { $Adapters.StartOwnedProcess } else {
                            {
                                param($FilePath, $ArgumentList, $WorkingDirectory, $Environment)
                                Import-Module (Join-Path $PSScriptRoot 'managed-process-job.psm1') -Force
                                $job = New-ManagedProcessJob -SessionId ([Guid]::NewGuid())
                                try {
                                    Start-ManagedProcess -Job $job -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory | Out-Null
                                    return $job
                                } catch { Stop-ManagedProcessJob -Job $job -GraceSeconds 0; throw }
                            }
                        }
                        $arguments = @('run', '--no-sync', 'python', '-m', 'irodori_openai_tts', '--host', '127.0.0.1', '--port', '8088')
                        $owned = & $startOwned $uvPath $arguments $serverPath $runtimeEnvironment
                        $sleep = if ($Adapters.ContainsKey('Sleep')) { $Adapters.Sleep } else { { param($Milliseconds) Start-Sleep -Milliseconds $Milliseconds } }
                        $healthy = $false
                        for ($attempt = 0; $attempt -lt 180; $attempt++) {
                            if (Test-IrodoriBootstrapHealth -InvokeHttp $invokeHttp -Port 8088) { $healthy = $true; break }
                            & $sleep 500
                        }
                        if (-not $healthy) { throw 'Irodori /health did not become ready.' }
                    }
                    $voicesResponse = & $invokeHttp 'GET' 'http://127.0.0.1:8088/v1/audio/voices' $null
                    if ($null -eq $voicesResponse -or [int] $voicesResponse.status_code -ne 200) { throw 'Irodori voice list request failed.' }
                    $voices = @($voicesResponse.body.data)
                    if ($voices.Count -eq 0 -or [string]::IsNullOrWhiteSpace([string] $voices[0].id)) {
                        $status = 'ready_without_voice'
                    } else {
                        $speechBody = [ordered]@{ model = 'irodori-tts'; input = 'Irodori startup check.'; voice = [string] $voices[0].id; response_format = 'wav'; speed = 1.0 }
                        $speechResponse = & $invokeHttp 'POST' 'http://127.0.0.1:8088/v1/audio/speech' $speechBody
                        if ($null -eq $speechResponse -or [int] $speechResponse.status_code -ne 200 -or -not (Test-IrodoriWaveBytes -Bytes $speechResponse.bytes)) {
                            $status = 'warmup_failed'
                        } else { $status = 'ready' }
                    }
                }
            }
        } catch [OperationCanceledException] { throw } catch [Management.Automation.PipelineStoppedException] { throw } catch {
            $status = 'setup_failed'
            if ($Adapters.ContainsKey('WriteProgress')) { & $Adapters.WriteProgress 'error' $_.Exception.Message } else { Write-Warning "Irodori setup failed; continuing without managed TTS: $($_.Exception.Message)" }
        }

        $appExitCode = & $runApp
        return [pscustomobject]@{ status = $status; app_exit_code = [int] $appExitCode; backend = $backend }
    } finally {
        if ($null -ne $owned) {
            $stopOwned = if ($Adapters.ContainsKey('StopOwnedProcess')) { $Adapters.StopOwnedProcess } else {
                { param($Owned) Stop-ManagedProcessJob -Job $Owned -GraceSeconds 5 }
            }
            & $stopOwned $owned
        }
        foreach ($name in $managedEnvironmentNames) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name]) }
    }
}

Export-ModuleMember -Function Import-IrodoriManifest, Get-IrodoriLayout, Get-IrodoriBackend, Test-IrodoriCompletion, Invoke-IrodoriProvision, Test-IrodoriRuntimeReady, Invoke-IrodoriBootstrap
