[CmdletBinding()]
param(
    [string]$DurationMinutes = "120",
    [string]$SampleSeconds = "5",
    [string]$OutputDir = "artifacts/soak",
    [string]$Executable = "target/debug/parallel-world-desktop.exe",
    [string[]]$ArgumentList = @(),
    [string]$DiagnosticsHeartbeat = "",
    [switch]$FaultInjection,
    [string]$FaultTarget = "None",
    [switch]$ConfirmOwnedFault,
    [switch]$SelfTest,
    [switch]$SelfTestRootChild,
    [string]$Seed = "424242"
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$ExitSuccess = 0
$ExitUsage = 2
$ExitUnexpected = 3
$ExitThreshold = 4
$ExitArtifact = 5
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$startedProcess = $null
$selfTestLateMarker = ""
$selfTestSpawnLog = ""

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace ParallelWorld.Soak {
    public static class ProcessTree {
        private const uint SnapshotProcesses = 0x00000002;
        private static readonly IntPtr InvalidHandle = new IntPtr(-1);

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
        private struct ProcessEntry32 {
            public uint Size;
            public uint Usage;
            public uint ProcessId;
            public IntPtr DefaultHeapId;
            public uint ModuleId;
            public uint Threads;
            public uint ParentProcessId;
            public int BasePriority;
            public uint Flags;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)]
            public string Executable;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint processId);
        [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
        private static extern bool Process32First(IntPtr snapshot, ref ProcessEntry32 entry);
        [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
        private static extern bool Process32Next(IntPtr snapshot, ref ProcessEntry32 entry);
        [DllImport("kernel32.dll")]
        private static extern bool CloseHandle(IntPtr handle);

        public static Dictionary<int, int> ParentMap() {
            var result = new Dictionary<int, int>();
            IntPtr snapshot = CreateToolhelp32Snapshot(SnapshotProcesses, 0);
            if (snapshot == InvalidHandle) return result;
            try {
                var entry = new ProcessEntry32();
                entry.Size = (uint)Marshal.SizeOf(typeof(ProcessEntry32));
                if (!Process32First(snapshot, ref entry)) return result;
                do {
                    result[(int)entry.ProcessId] = (int)entry.ParentProcessId;
                    entry.Size = (uint)Marshal.SizeOf(typeof(ProcessEntry32));
                } while (Process32Next(snapshot, ref entry));
                return result;
            }
            finally { CloseHandle(snapshot); }
        }
    }
}
'@

function Get-ProcessParentMap {
    $map = @{}
    try {
        foreach ($process in @(Get-CimInstance Win32_Process -ErrorAction Stop)) {
            $map[[int]$process.ProcessId] = [int]$process.ParentProcessId
        }
    }
    catch {
        foreach ($pair in [ParallelWorld.Soak.ProcessTree]::ParentMap().GetEnumerator()) {
            $map[[int]$pair.Key] = [int]$pair.Value
        }
    }
    return $map
}

function Get-ProcessIdentity([int]$ProcessId) {
    try {
        $process = Get-Process -Id $ProcessId -ErrorAction Stop
        $process.Refresh()
        return [pscustomobject]@{
            ProcessId = [int]$process.Id
            StartTicks = [long]$process.StartTime.ToUniversalTime().Ticks
        }
    }
    catch {
        return $null
    }
}

function Test-ProcessIdentity([object]$Identity) {
    if ($null -eq $Identity) { return $false }
    $current = Get-ProcessIdentity ([int]$Identity.ProcessId)
    return $null -ne $current -and [long]$current.StartTicks -eq [long]$Identity.StartTicks
}

function Stop-ProcessIdentity([object]$Identity) {
    $process = Get-MatchingProcess $Identity
    if ($null -ne $process) {
        try { $process.Kill() } catch { }
    }
}

function Get-MatchingProcess([object]$Identity) {
    if ($null -eq $Identity) { return $null }
    try {
        $process = Get-Process -Id ([int]$Identity.ProcessId) -ErrorAction Stop
        $process.Refresh()
        if ([long]$process.StartTime.ToUniversalTime().Ticks -ne [long]$Identity.StartTicks) { return $null }
        return $process
    }
    catch { return $null }
}

function Test-OwnedProcessIdentity([object]$Identity, [object]$RootIdentity) {
    return (Test-ProcessIdentity $RootIdentity) -and
        (Test-ProcessIdentity $Identity) -and
        (Test-IsStrictDescendant ([int]$Identity.ProcessId) ([int]$RootIdentity.ProcessId))
}

function Stop-OwnedProcessIdentity([object]$Identity, [object]$RootIdentity) {
    if (-not (Test-OwnedProcessIdentity $Identity $RootIdentity)) { return $false }
    $process = Get-MatchingProcess $Identity
    if ($null -eq $process -or -not (Test-IsStrictDescendant ([int]$Identity.ProcessId) ([int]$RootIdentity.ProcessId))) { return $false }
    try { $process.Kill(); return $true } catch { return $false }
}

function Get-ProcessDepth([int]$ProcessId, [int]$RootPid, [hashtable]$Parents) {
    $current = $ProcessId
    for($depth = 0; $depth -lt 64; $depth++) {
        if($current -eq $RootPid) { return $depth }
        if(-not $Parents.ContainsKey($current)) { return -1 }
        $current = [int]$Parents[$current]
    }
    return -1
}

function Get-OwnedProcessSnapshot([object]$RootIdentity) {
    if(-not (Test-ProcessIdentity $RootIdentity)){return @()}
    $rootPid=[int]$RootIdentity.ProcessId
    $parents=Get-ProcessParentMap
    $identities=@{}
    foreach($processId in @($rootPid)+@(Get-DescendantIds $rootPid)){
        $identity=Get-ProcessIdentity ([int]$processId)
        if($null-ne$identity){$identities[[int]$identity.ProcessId]=$identity}
    }
    $entries=@()
    foreach($identity in $identities.Values){
        if([int]$identity.ProcessId-eq$rootPid){continue}
        $lineage=@($identity);$current=[int]$identity.ProcessId;$valid=$false
        for($depth=1;$depth-lt64;$depth++){
            if(-not$parents.ContainsKey($current)){break}
            $parent=[int]$parents[$current]
            if(-not$identities.ContainsKey($parent)){break}
            $lineage+=,$identities[$parent]
            if($parent-eq$rootPid){$valid=$true;break}
            $current=$parent
        }
        if($valid){$entries+=,[pscustomobject]@{Identity=$identity;Depth=$lineage.Count-1;Lineage=$lineage}}
    }
    return @($entries)
}

function Test-CapturedOwnershipLineage([object]$Entry,[object]$RootIdentity) {
    if($null-eq$Entry-or$Entry.Lineage.Count-lt2-or-not(Test-ProcessIdentity $RootIdentity)){return $false}
    $parents=Get-ProcessParentMap
    for($index=0;$index-lt$Entry.Lineage.Count;$index++){
        $identity=$Entry.Lineage[$index]
        if(-not(Test-ProcessIdentity $identity)){return $false}
        if($index-lt$Entry.Lineage.Count-1){
            if(-not$parents.ContainsKey([int]$identity.ProcessId)-or[int]$parents[[int]$identity.ProcessId]-ne[int]$Entry.Lineage[$index+1].ProcessId){return $false}
        }
    }
    $capturedRoot=$Entry.Lineage[$Entry.Lineage.Count-1]
    return [int]$capturedRoot.ProcessId-eq[int]$RootIdentity.ProcessId-and[long]$capturedRoot.StartTicks-eq[long]$RootIdentity.StartTicks
}

function Stop-CapturedOwnedIdentity([object]$Entry,[object]$RootIdentity) {
    if(-not(Test-CapturedOwnershipLineage $Entry $RootIdentity)){return $false}
    $process=Get-MatchingProcess $Entry.Identity
    if($null-eq$process-or-not(Test-CapturedOwnershipLineage $Entry $RootIdentity)){return $false}
    try{$process.Kill();return $true}catch{return $false}
}

function Stop-ProcessTree([object]$RootIdentity, [hashtable]$CapturedByKey) {
    if (-not (Test-ProcessIdentity $RootIdentity)) { return }
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    do{
        $owned=@(Get-OwnedProcessSnapshot $RootIdentity|Sort-Object Depth -Descending)
        foreach($entry in $owned){$identity=$entry.Identity;$CapturedByKey[([string]$identity.ProcessId+":"+[string]$identity.StartTicks)]=$identity}
        if($owned.Count-eq0){break}
        foreach($entry in $owned){[void](Stop-CapturedOwnedIdentity $entry $RootIdentity)}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow-lt$deadline-and(Test-ProcessIdentity $RootIdentity))
    Stop-ProcessIdentity $RootIdentity
}

function Get-ResidualProcessIdentities([hashtable]$CapturedByKey) {
    $parents = Get-ProcessParentMap
    $ownedPids = @{}
    foreach($identity in $CapturedByKey.Values){$ownedPids[[int]$identity.ProcessId]=$true}
    $residual = @{}
    $changed = $true
    while($changed){
        $changed = $false
        foreach($pidValue in $parents.Keys){
            $processId=[int]$pidValue;$parent=[int]$parents[$processId]
            if($ownedPids.ContainsKey($parent) -and -not $ownedPids.ContainsKey($processId)){
                $identity=Get-ProcessIdentity $processId
                if($null -ne $identity){$ownedPids[$processId]=$true;$residual[([string]$identity.ProcessId+":"+[string]$identity.StartTicks)]=$identity;$changed=$true}
            }
        }
    }
    foreach($identity in $CapturedByKey.Values){if(Test-ProcessIdentity $identity){$residual[([string]$identity.ProcessId+":"+[string]$identity.StartTicks)]=$identity}}
    return @($residual.Values)
}

function Wait-ResidualProcessIdentities([hashtable]$CapturedByKey, [int]$TimeoutMilliseconds) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        $residual = @(Get-ResidualProcessIdentities $CapturedByKey)
        if($residual.Count -eq 0){return @()}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $deadline)
    return @(Get-ResidualProcessIdentities $CapturedByKey)
}

function Get-DescendantIds([int]$RootPid) {
    $result = @()
    $parents = Get-ProcessParentMap
    $pending = @($RootPid)
    while ($pending.Count -gt 0) {
        $parent = [int]$pending[0]
        if ($pending.Count -eq 1) { $pending = @() } else { $pending = @($pending[1..($pending.Count - 1)]) }
        $children = @($parents.Keys | Where-Object { [int]$parents[$_] -eq $parent })
        foreach ($childPid in $children) { $result += [int]$childPid; $pending += [int]$childPid }
    }
    return $result
}

function Get-ProcessTreeStats([int]$RootPid) {
    $ids = @($RootPid) + @(Get-DescendantIds $RootPid)
    $rss = 0L; $private = 0L; $handles = 0L; $threads = 0L; $live = @(); $identities = @()
    foreach ($id in $ids) {
        $process = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($null -ne $process) {
            $process.Refresh(); $live += $id; $rss += [long]$process.WorkingSet64
            $private += [long]$process.PrivateMemorySize64; $handles += [long]$process.HandleCount
            $threads += [long]$process.Threads.Count
            $identities += [pscustomobject]@{ ProcessId = [int]$process.Id; StartTicks = [long]$process.StartTime.ToUniversalTime().Ticks }
        }
    }
    return @{ ProcessIds = $live; ProcessIdentities = $identities; Rss = $rss; Private = $private; Handles = $handles; Threads = $threads }
}

function Test-IsDescendant([int]$ProcessId, [int]$RootPid) {
    $parents = Get-ProcessParentMap
    $current = $ProcessId
    for($depth=0;$depth -lt 32;$depth++) {
        if($current -eq $RootPid){return $true}
        if(-not $parents.ContainsKey($current) -or [int]$parents[$current] -eq 0){return $false}
        $current = [int]$parents[$current]
    }
    return $false
}

function Test-IsStrictDescendant([int]$ProcessId, [int]$RootPid) {
    return $ProcessId -ne $RootPid -and (Test-IsDescendant $ProcessId $RootPid)
}

function Get-HeartbeatLong([object]$Heartbeat, [string]$Name, [long]$Fallback) {
    if ($null -ne $Heartbeat -and $null -ne $Heartbeat.PSObject.Properties[$Name]) {
        return [long]$Heartbeat.$Name
    }
    return $Fallback
}

function ConvertTo-WindowsCommandLineArgument([string]$Value) {
    if ($Value -notmatch '[\s"]' -and $Value.Length -gt 0) { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"'); $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') { $slashes++; continue }
        if ($character -eq '"') { [void]$builder.Append(('\' * ($slashes * 2 + 1))); [void]$builder.Append('"') }
        else { [void]$builder.Append(('\' * $slashes)); [void]$builder.Append($character) }
        $slashes = 0
    }
    [void]$builder.Append(('\' * ($slashes * 2))); [void]$builder.Append('"')
    return $builder.ToString()
}

function Join-WindowsCommandLine([string[]]$Values) {
    return (($Values | ForEach-Object { ConvertTo-WindowsCommandLineArgument $_ }) -join ' ')
}

function Write-JsonLine([string]$Path, [object]$Value) {
    $line = ($Value | ConvertTo-Json -Compress -Depth 8) + [Environment]::NewLine
    [System.IO.File]::AppendAllText($Path, $line, $Utf8NoBom)
}

function Get-DirectoryStats([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return @{ Count = 0; Bytes = 0L }
    }
    $files = @(Get-ChildItem -LiteralPath $Path -File -Recurse -ErrorAction SilentlyContinue)
    $bytes = 0L
    foreach ($file in $files) { $bytes += [long]$file.Length }
    return @{ Count = $files.Count; Bytes = $bytes }
}

function Get-SlopePerHour([object[]]$Samples, [string]$Property) {
    if ($Samples.Count -lt 2) { return 0.0 }
    $origin = [double]$Samples[0].timestamp_ms
    $xs = @()
    $ys = @()
    foreach ($sample in $Samples) {
        $xs += (([double]$sample.timestamp_ms - $origin) / 3600000.0)
        $ys += [double]$sample.$Property
    }
    $meanX = ($xs | Measure-Object -Average).Average
    $meanY = ($ys | Measure-Object -Average).Average
    $numerator = 0.0
    $denominator = 0.0
    for ($i = 0; $i -lt $xs.Count; $i++) {
        $numerator += ($xs[$i] - $meanX) * ($ys[$i] - $meanY)
        $denominator += [Math]::Pow(($xs[$i] - $meanX), 2)
    }
    if ($denominator -eq 0.0) { return 0.0 }
    return $numerator / $denominator
}

function Get-Maximum([object[]]$Samples, [string]$Property) {
    if ($Samples.Count -eq 0) { return 0L }
    return [long](($Samples | Measure-Object -Property $Property -Maximum).Maximum)
}

function Get-StabilityThresholds {
    return [ordered]@{
        rss_slope_bytes_per_hour = 67108864; private_slope_bytes_per_hour = 67108864
        rss_growth_bytes = 67108864; private_growth_bytes = 67108864
        handle_slope_per_hour = 60; handle_growth = 100
        thread_slope_per_hour = 6; thread_growth = 8
        queue_max = 64; dropped_max = 100; cache_file_max = 1000; log_bytes_max = 268435456
        restart_max = 8; fault_max = 32
        unexpected_exit_max = 0; panic_max = 0; orphan_max = 0
    }
}

function Test-SupervisorUnhealthyViolation([object[]]$Samples, [double]$Duration) {
    if ($Duration -lt 120) { return $false }
    return @($Samples | Where-Object { $null -ne $_.supervisor_healthy -and $_.supervisor_healthy -eq $false }).Count -gt 0
}

try {
    $durationMinutesValue = 0.0
    $sampleSecondsValue = 0
    $seedValue = 0
    $culture = [Globalization.CultureInfo]::InvariantCulture
    if (-not [double]::TryParse($DurationMinutes, [Globalization.NumberStyles]::Float, $culture, [ref]$durationMinutesValue) -or
        -not [int]::TryParse($SampleSeconds, [Globalization.NumberStyles]::Integer, $culture, [ref]$sampleSecondsValue) -or
        -not [int]::TryParse($Seed, [Globalization.NumberStyles]::Integer, $culture, [ref]$seedValue) -or
        @("None", "OwnedChild") -notcontains $FaultTarget) {
        [Console]::Error.WriteLine("Invalid argument value. DurationMinutes, SampleSeconds, Seed, and FaultTarget must use supported values.")
        exit $ExitUsage
    }
    if ($SelfTest) {
        $thresholdProbe = Get-StabilityThresholds
        foreach ($requiredThreshold in @("handle_slope_per_hour", "handle_growth", "thread_slope_per_hour", "thread_growth", "restart_max", "fault_max")) {
            if ($null -eq $thresholdProbe[$requiredThreshold]) { throw "Self-test threshold missing: $requiredThreshold" }
        }
        $unhealthyProbe = @([pscustomobject]@{ supervisor_healthy = $false })
        if (-not (Test-SupervisorUnhealthyViolation $unhealthyProbe 120)) { throw "Two-hour acceptance must reject an unhealthy supervisor sample." }
        $durationMinutesValue = 0.12; $sampleSecondsValue = 1
        $Executable = "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe"
        $OutputDir = Join-Path ([IO.Path]::GetTempPath()) ("parallel-world-soak-selftest-" + $PID)
        [IO.Directory]::CreateDirectory($OutputDir) | Out-Null
        $identityProbe = Start-Process -FilePath $Executable -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 30"' -PassThru -WindowStyle Hidden
        try {
            $staleIdentity = [pscustomobject]@{ ProcessId = $identityProbe.Id; StartTicks = $identityProbe.StartTime.ToUniversalTime().Ticks + 1 }
            Stop-ProcessIdentity $staleIdentity
            $identityProbe.Refresh()
            if ($identityProbe.HasExited) { throw "Identity-safe cleanup killed a reused PID." }
        }
        finally {
            Stop-Process -Id $identityProbe.Id -Force -ErrorAction SilentlyContinue
        }
        $DiagnosticsHeartbeat = Join-Path $OutputDir "heartbeat.json"
        $helperPath = Join-Path $OutputDir "heartbeat-supervisor.ps1"
        $lateSpawnerPath = Join-Path $OutputDir "late-grandchild.ps1"
        $selfTestLateMarker = Join-Path $OutputDir "late-grandchild.json"
        $selfTestSpawnLog = Join-Path $OutputDir "supervisor-spawns.jsonl"
        $lateSpawnerSource = @'
param([string]$Marker)
$grandchild=Start-Process powershell.exe -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 30"' -PassThru -WindowStyle Hidden
$value=@{ProcessId=$grandchild.Id;StartTicks=$grandchild.StartTime.ToUniversalTime().Ticks}
[IO.File]::WriteAllText($Marker,($value|ConvertTo-Json -Compress),(New-Object Text.UTF8Encoding($false)))
Start-Sleep -Seconds 30
'@
        [IO.File]::WriteAllText($lateSpawnerPath, $lateSpawnerSource, $Utf8NoBom)
        $helperSource = @'
param([string]$Heartbeat,[switch]$IncludeRoot,[string]$LateSpawner,[string]$LateMarker,[string]$SpawnLog)
$restart=0;$fault=0;$child=$null
$started=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();$runId="$PID-$started";$watch=[Diagnostics.Stopwatch]::StartNew();$late=$null;Start-Sleep -Seconds 2
while($true){
 if($null -eq $child -or $child.HasExited){if($null -ne $child){$restart++;$fault++};$child=Start-Process powershell.exe -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 30"' -PassThru -WindowStyle Hidden;$spawn=@{ProcessId=$child.Id;StartTicks=$child.StartTime.ToUniversalTime().Ticks};[IO.File]::AppendAllText($SpawnLog,($spawn|ConvertTo-Json -Compress)+[Environment]::NewLine,(New-Object Text.UTF8Encoding($false)))}
 if($null -eq $late -and $watch.ElapsedMilliseconds -ge 6500){$late=Start-Process powershell.exe -ArgumentList @('-NoProfile','-File',$LateSpawner,'-Marker',$LateMarker) -PassThru -WindowStyle Hidden}
 $childIds=if($IncludeRoot){@($PID,$child.Id)}else{@($child.Id)}
 $value=@{schema_version=1;process_id=$PID;run_id=$runId;started_timestamp_ms=$started;timestamp_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();audio_device='self-test-device';supervisor_healthy=$true;input_queue_depth=7;output_queue_depth=8;dropped_items=2;cache_file_count=9;log_bytes=1024;restart_count=$restart;panic_count=0;fault_count=$fault;child_process_ids=$childIds}
 $tmp=$Heartbeat+'.tmp';[IO.File]::WriteAllText($tmp,($value|ConvertTo-Json -Compress),(New-Object Text.UTF8Encoding($false)));Move-Item -LiteralPath $tmp -Destination $Heartbeat -Force
 Start-Sleep -Milliseconds 200
}
'@
        [IO.File]::WriteAllText($helperPath, $helperSource, $Utf8NoBom)
        $stale = @{schema_version=1;process_id=1;run_id="previous-run";started_timestamp_ms=1;timestamp_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();audio_device="stale";supervisor_healthy=$true;input_queue_depth=0;output_queue_depth=0;dropped_items=0;cache_file_count=0;log_bytes=0;restart_count=0;panic_count=0;fault_count=0;child_process_ids=@()}
        [IO.File]::WriteAllText($DiagnosticsHeartbeat, ($stale | ConvertTo-Json -Compress), $Utf8NoBom)
        $ArgumentList = @("-NoProfile", "-File", $helperPath, "-Heartbeat", $DiagnosticsHeartbeat, "-LateSpawner", $lateSpawnerPath, "-LateMarker", $selfTestLateMarker, "-SpawnLog", $selfTestSpawnLog)
        if($SelfTestRootChild){$ArgumentList += "-IncludeRoot"}
        $FaultInjection = -not $SelfTestRootChild; $FaultTarget = "OwnedChild"; $ConfirmOwnedFault = $true
    }
    if ($durationMinutesValue -le 0 -or $sampleSecondsValue -lt 1 -or $sampleSecondsValue -gt 5) {
        [Console]::Error.WriteLine("DurationMinutes must be positive and SampleSeconds must be between 1 and 5.")
        exit $ExitUsage
    }
    if ($FaultInjection -and ($FaultTarget -ne "OwnedChild" -or -not $ConfirmOwnedFault)) {
        [Console]::Error.WriteLine("FaultInjection requires -FaultTarget OwnedChild -ConfirmOwnedFault; only harness-owned descendants may be killed.")
        exit $ExitUsage
    }
    $resolvedExecutable = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Executable)
    if (-not (Test-Path -LiteralPath $resolvedExecutable -PathType Leaf)) {
        [Console]::Error.WriteLine("Executable not found: $resolvedExecutable")
        exit $ExitUsage
    }
    $resolvedOutput = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDir)
    [System.IO.Directory]::CreateDirectory($resolvedOutput) | Out-Null
    $runId = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ") + "-" + $seedValue
    $jsonlPath = Join-Path $resolvedOutput ($runId + ".jsonl")
    $summaryPath = Join-Path $resolvedOutput ($runId + "-summary.json")
    [System.IO.File]::WriteAllText($jsonlPath, "", $Utf8NoBom)

    $gitHash = (& git rev-parse HEAD 2>$null)
    if (-not $gitHash) { $gitHash = "unknown" }
    $gitDirty = [bool](& git status --porcelain 2>$null)
    $executableSha256 = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash
    $os = [Environment]::OSVersion.VersionString
    $audioDevice = "unknown"
    $startedAt = [DateTimeOffset]::UtcNow
    $deadline = $startedAt.AddMinutes($durationMinutesValue)
    $faultTimeline = @()
    $faultRecorded = $false
    $faultVictim = 0
    $faultReplacementSeen = $false
    $faultRecovered = $false
    $faultDeadline = $null
    $samples = @()

    $serializedArguments = Join-WindowsCommandLine $ArgumentList
    $startedProcess = Start-Process -FilePath $resolvedExecutable -ArgumentList $serializedArguments -PassThru -WindowStyle Hidden
    $rootIdentity = Get-ProcessIdentity $startedProcess.Id
    if ($null -eq $rootIdentity) { throw "Started process identity could not be captured." }
    Write-JsonLine $jsonlPath ([ordered]@{
        type = "metadata"; schema_version = 1; run_id = $runId; build_hash = $gitHash;
        os = $os; audio_device = $audioDevice; seed = $seedValue; fault_injection = [bool]$FaultInjection;
        executable_sha256 = $executableSha256; git_dirty = $gitDirty;
        started_at = $startedAt.ToString("o"); executable = $resolvedExecutable; arguments = $ArgumentList; process_id = $startedProcess.Id
    })

    $unexpectedExit = 0
    $heartbeatSeen = $false
    $heartbeatInvalid = $false
    $metadataUpdated = $false
    $lastFreshHeartbeat = $null
    $faultPrePidSet = @()
    $faultPreIdentityKeys = @()
    $faultPreRestarts = 0
    $faultPreFaults = 0
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        $startedProcess.Refresh()
        if ($startedProcess.HasExited) { $unexpectedExit = 1; break }
        $now = [DateTimeOffset]::UtcNow
        $tree = Get-ProcessTreeStats $startedProcess.Id
        $cache = Get-DirectoryStats (Join-Path (Split-Path $resolvedOutput -Parent) "cache")
        $logs = Get-DirectoryStats (Join-Path (Split-Path $resolvedOutput -Parent) "logs")
        $heartbeat = $null
        if ($DiagnosticsHeartbeat -and (Test-Path -LiteralPath $DiagnosticsHeartbeat)) {
            try {
                $candidate = Get-Content -LiteralPath $DiagnosticsHeartbeat -Raw | ConvertFrom-Json
                $mandatory = @("schema_version","process_id","run_id","started_timestamp_ms","timestamp_ms","audio_device","supervisor_healthy","input_queue_depth","output_queue_depth","dropped_items","cache_file_count","log_bytes","restart_count","panic_count","fault_count","child_process_ids")
                $complete = $true; foreach($field in $mandatory){if($null -eq $candidate.PSObject.Properties[$field]){$complete=$false}}
                foreach($field in @("process_id","started_timestamp_ms","timestamp_ms","input_queue_depth","output_queue_depth","dropped_items","cache_file_count","log_bytes","restart_count","panic_count","fault_count")){if($complete -and [long]$candidate.$field -lt 0){$complete=$false}}
                $age = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - [long]$candidate.timestamp_ms
                $fresh = $age -ge 0 -and $age -le [Math]::Max(3 * $sampleSecondsValue * 1000, 10000)
                $pidsOwned = @($candidate.child_process_ids | Where-Object { -not (Test-IsStrictDescendant ([int]$_) $startedProcess.Id) }).Count -eq 0
                $currentRun = [int]$candidate.process_id -eq $startedProcess.Id -and [long]$candidate.timestamp_ms -ge $startedAt.ToUnixTimeMilliseconds() -and [long]$candidate.started_timestamp_ms -ge $startedAt.ToUnixTimeMilliseconds()
                if($candidate.schema_version -eq 1 -and $complete -and $fresh -and $pidsOwned -and $currentRun){$heartbeat=$candidate;$lastFreshHeartbeat=$candidate;$heartbeatSeen=$true;$audioDevice=[string]$candidate.audio_device}
                elseif($heartbeatSeen -or $now -ge $startedAt.AddSeconds(10)){$heartbeatInvalid=$true}
            } catch { if($heartbeatSeen -or $now -ge $startedAt.AddSeconds(10)){$heartbeatInvalid=$true}; $heartbeat = $null }
        }
        elseif($now -ge $startedAt.AddSeconds(10)){$heartbeatInvalid=$true}
        if($heartbeatSeen -and -not $metadataUpdated){Write-JsonLine $jsonlPath ([ordered]@{type="metadata_update";timestamp=$now.ToString("o");audio_device=$audioDevice});$metadataUpdated=$true}
        if ($FaultInjection -and -not $faultRecorded -and $now -ge $startedAt.AddMinutes($durationMinutesValue / 2.0)) {
            if($null -eq $lastFreshHeartbeat){throw "Fresh heartbeat required before fault injection."}
            $faultTree = Get-ProcessTreeStats $startedProcess.Id
            $declaredPids = @($lastFreshHeartbeat.child_process_ids | ForEach-Object { [int]$_ })
            $ownedChildren = @($faultTree.ProcessIdentities | Where-Object { $declaredPids -contains [int]$_.ProcessId -and (Test-OwnedProcessIdentity $_ $rootIdentity) })
            if($ownedChildren.Count -eq 0){throw "No identity-verified owned child is available for fault injection."}
            $faultPrePidSet=@($faultTree.ProcessIds)+@($lastFreshHeartbeat.child_process_ids)
            $faultPreIdentityKeys=@($faultTree.ProcessIdentities | ForEach-Object { [string]$_.ProcessId + ":" + [string]$_.StartTicks })
            $faultPreRestarts=[long]$lastFreshHeartbeat.restart_count;$faultPreFaults=[long]$lastFreshHeartbeat.fault_count
            $faultVictimIdentity=$ownedChildren[0];$faultVictim=[int]$faultVictimIdentity.ProcessId
            if(-not (Stop-OwnedProcessIdentity $faultVictimIdentity $rootIdentity)){throw "Owned child identity changed before fault injection."}
            $faultDeadline=$now.AddSeconds(10)
            $fault=[ordered]@{timestamp=$now.ToString("o");timestamp_ms=$now.ToUnixTimeMilliseconds();kind="owned-child-kill";process_id=$faultVictim;start_ticks=$faultVictimIdentity.StartTicks;pre_pid_set=$faultPrePidSet;pre_restart_count=$faultPreRestarts;pre_fault_count=$faultPreFaults;seed=$seedValue;replacement_deadline=$faultDeadline.ToString("o")}
            $faultTimeline+=$fault;Write-JsonLine $jsonlPath ([ordered]@{type="fault";value=$fault});$faultRecorded=$true
        }
        if($faultRecorded -and $null -ne $heartbeat){$replacement=@($heartbeat.child_process_ids|ForEach-Object{Get-ProcessIdentity ([int]$_)}|Where-Object{$null -ne $_ -and (Test-OwnedProcessIdentity $_ $rootIdentity) -and $faultPreIdentityKeys -notcontains ([string]$_.ProcessId + ":" + [string]$_.StartTicks)});$faultReplacementSeen=$replacement.Count -gt 0;if($faultReplacementSeen -and [long]$heartbeat.timestamp_ms -gt [long]$fault.timestamp_ms -and [long]$heartbeat.restart_count -gt $faultPreRestarts -and [long]$heartbeat.fault_count -gt $faultPreFaults -and $heartbeat.supervisor_healthy -eq $true -and $now -le $faultDeadline){$faultRecovered=$true}}
        $diagnosticChildIds = @()
        if ($null -ne $heartbeat -and $null -ne $heartbeat.PSObject.Properties["child_process_ids"]) { $diagnosticChildIds = @($heartbeat.child_process_ids) }
        $diagnosticChildIdentities = @($diagnosticChildIds | ForEach-Object { Get-ProcessIdentity ([int]$_) } | Where-Object { $null -ne $_ })
        $sample = [pscustomobject][ordered]@{
            type = "sample"; timestamp = $now.ToString("o"); timestamp_ms = $now.ToUnixTimeMilliseconds();
            rss_bytes = [long]$tree.Rss; private_bytes = [long]$tree.Private;
            handle_count = [long]$tree.Handles; thread_count = [long]$tree.Threads; process_ids = $tree.ProcessIds; process_identities = $tree.ProcessIdentities; diagnostic_child_process_ids = $diagnosticChildIds; diagnostic_child_process_identities = $diagnosticChildIdentities;
            input_queue_depth = Get-HeartbeatLong $heartbeat "input_queue_depth" -1; output_queue_depth = Get-HeartbeatLong $heartbeat "output_queue_depth" -1; dropped_items = Get-HeartbeatLong $heartbeat "dropped_items" -1;
            cache_file_count = Get-HeartbeatLong $heartbeat "cache_file_count" ([long]$cache.Count); log_bytes = Get-HeartbeatLong $heartbeat "log_bytes" ([long]$logs.Bytes);
            restart_count = Get-HeartbeatLong $heartbeat "restart_count" -1; fault_count = Get-HeartbeatLong $heartbeat "fault_count" ([long]$faultTimeline.Count);
            unexpected_exit_count = 0; panic_count = Get-HeartbeatLong $heartbeat "panic_count" -1; orphan_process_count = 0;
            supervisor_healthy = if($null -ne $heartbeat){[bool]$heartbeat.supervisor_healthy}else{$null};
            diagnostics = $heartbeat
        }
        $samples += $sample
        Write-JsonLine $jsonlPath $sample
        Start-Sleep -Seconds $sampleSecondsValue
    }

    $steady = @($samples | Where-Object { $_.timestamp_ms -ge ($startedAt.ToUnixTimeMilliseconds() + 120000) })
    $violations = @()
    $thresholds = Get-StabilityThresholds
    $slopes = [ordered]@{}
    $growth = [ordered]@{}
    if ($steady.Count -ge 2) {
        foreach ($property in @("rss_bytes","private_bytes","handle_count","thread_count","input_queue_depth","output_queue_depth","cache_file_count","log_bytes")) {
            $slopes[$property] = Get-SlopePerHour $steady $property
            $growth[$property] = [long]$steady[-1].$property - [long]$steady[0].$property
        }
        if ($slopes.rss_bytes -gt $thresholds.rss_slope_bytes_per_hour) { $violations += "rss_slope" }
        if ($slopes.private_bytes -gt $thresholds.private_slope_bytes_per_hour) { $violations += "private_bytes_slope" }
        if ($growth.rss_bytes -gt $thresholds.rss_growth_bytes) { $violations += "rss_growth" }
        if ($growth.private_bytes -gt $thresholds.private_growth_bytes) { $violations += "private_bytes_growth" }
        if ($slopes.handle_count -gt $thresholds.handle_slope_per_hour) { $violations += "handle_slope" }
        if ($growth.handle_count -gt $thresholds.handle_growth) { $violations += "handle_growth" }
        if ($slopes.thread_count -gt $thresholds.thread_slope_per_hour) { $violations += "thread_slope" }
        if ($growth.thread_count -gt $thresholds.thread_growth) { $violations += "thread_growth" }
    }
    if ($unexpectedExit -ne 0) { $violations += "unexpected_exit" }
    $maximums = [ordered]@{}
    foreach ($property in @("rss_bytes","private_bytes","handle_count","thread_count","input_queue_depth","output_queue_depth","dropped_items","cache_file_count","log_bytes","restart_count","fault_count","panic_count")) { $maximums[$property] = Get-Maximum $samples $property }
    if ($maximums.input_queue_depth -gt $thresholds.queue_max -or $maximums.output_queue_depth -gt $thresholds.queue_max) { $violations += "queue_cap" }
    if ($maximums.dropped_items -gt $thresholds.dropped_max) { $violations += "dropped_cap" }
    if ($maximums.cache_file_count -gt $thresholds.cache_file_max) { $violations += "cache_file_cap" }
    if ($maximums.log_bytes -gt $thresholds.log_bytes_max) { $violations += "log_bytes_cap" }
    if ($maximums.restart_count -gt $thresholds.restart_max) { $violations += "restart_cap" }
    if ($maximums.fault_count -gt $thresholds.fault_max) { $violations += "fault_cap" }
    if ($maximums.panic_count -gt 0) { $violations += "panic" }
    if ($durationMinutesValue -ge 120 -and -not $heartbeatSeen) { $violations += "heartbeat_missing" }
    if ($SelfTest -and -not $heartbeatSeen) { $violations += "selftest_heartbeat_missing" }
    if (Test-SupervisorUnhealthyViolation $samples $durationMinutesValue) { $violations += "supervisor_unhealthy" }
    if ($heartbeatInvalid) { $violations += "heartbeat_stale_incomplete_or_unowned" }
    if ($FaultInjection -and -not $faultRecovered) { $violations += "fault_recovery_deadline_missed" }

    $knownIdentities = @($samples | ForEach-Object { $_.process_identities; $_.diagnostic_child_process_identities })
    $knownByKey = @{}
    foreach($identity in $knownIdentities){if($null -ne $identity){$knownByKey[([string]$identity.ProcessId + ":" + [string]$identity.StartTicks)] = $identity}}
    $knownByKey[([string]$rootIdentity.ProcessId + ":" + [string]$rootIdentity.StartTicks)]=$rootIdentity
    Stop-ProcessTree $rootIdentity $knownByKey
    $startedProcess = $null
    $orphanIds = @(Wait-ResidualProcessIdentities $knownByKey 5000 | ForEach-Object { $_.ProcessId } | Select-Object -Unique)
    if ($orphanIds.Count -gt 0) { $violations += "orphan_process" }
    if($SelfTest){
        if(-not (Test-Path -LiteralPath $selfTestLateMarker)){throw "Late-grandchild self-test marker was not created."}
        $lateIdentity=Get-Content -LiteralPath $selfTestLateMarker -Raw|ConvertFrom-Json
        if(Test-ProcessIdentity $lateIdentity){$orphanIds+= [int]$lateIdentity.ProcessId;$violations+="late_grandchild_orphan";Stop-ProcessIdentity $lateIdentity}
        $spawned=@(Get-Content -LiteralPath $selfTestSpawnLog|ForEach-Object{$_|ConvertFrom-Json})
        $minimumSpawns=if($SelfTestRootChild){2}else{3}
        if($spawned.Count -lt $minimumSpawns){$violations+="cleanup_respawn_not_observed"}
        foreach($spawnIdentity in $spawned){if(Test-ProcessIdentity $spawnIdentity){$orphanIds+=[int]$spawnIdentity.ProcessId;$violations+="cleanup_respawn_orphan";Stop-ProcessIdentity $spawnIdentity}}
    }
    $finishedAt = [DateTimeOffset]::UtcNow
    $supervisorUnhealthySeen = @($samples | Where-Object { $null -ne $_.supervisor_healthy -and $_.supervisor_healthy -eq $false }).Count -gt 0
    $summary = [ordered]@{
        schema_version = 1; run_id = $runId; passed = ($violations.Count -eq 0);
        build_hash = $gitHash; git_dirty=$gitDirty; executable_sha256=$executableSha256; os = $os; audio_device = $audioDevice; seed = $seedValue;
        started_at = $startedAt.ToString("o"); finished_at = $finishedAt.ToString("o");
        sample_count = $samples.Count; heartbeat_seen=$heartbeatSeen; supervisor_unhealthy_seen=$supervisorUnhealthySeen; fault_timeline = $faultTimeline; violations = $violations;
        slopes_per_hour=$slopes; growth=$growth; maximums=$maximums; thresholds=$thresholds; orphan_process_ids=$orphanIds;
        jsonl = $jsonlPath
    }
    [System.IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 8), $Utf8NoBom)
    Write-Output "Summary: $summaryPath"
    if ($unexpectedExit -ne 0) { exit $ExitUnexpected }
    if ($violations.Count -ne 0) { exit $ExitThreshold }
    exit $ExitSuccess
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit $ExitArtifact
}
finally {
    if ($null -ne $startedProcess) {
        $finalCleanup = @{}
        Stop-ProcessTree $rootIdentity $finalCleanup
    }
}
