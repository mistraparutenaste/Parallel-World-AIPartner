Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

if ($null -eq ('ParallelWorld.Runtime.ManagedProcessJobNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace ParallelWorld.Runtime {
    public sealed class JobHandle : SafeHandleZeroOrMinusOneIsInvalid {
        private JobHandle() : base(true) { }

        protected override bool ReleaseHandle() {
            return ManagedProcessJobNative.CloseNativeHandle(handle);
        }
    }

    public sealed class StartedProcessIdentity {
        public int ProcessId { get; private set; }
        public long StartTimeUtcTicks { get; private set; }

        public StartedProcessIdentity(int processId, long startTimeUtcTicks) {
            ProcessId = processId;
            StartTimeUtcTicks = startTimeUtcTicks;
        }
    }

    public static class ManagedProcessJobNative {
        private const uint KillOnJobClose = 0x00002000;
        private const uint CreateSuspended = 0x00000004;
        private const uint CreateNoWindow = 0x08000000;
        private const uint StartfUseShowWindow = 0x00000001;

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct StartupInformation {
            public uint Size;
            public string Reserved;
            public string Desktop;
            public string Title;
            public uint X;
            public uint Y;
            public uint XSize;
            public uint YSize;
            public uint XCountChars;
            public uint YCountChars;
            public uint FillAttribute;
            public uint Flags;
            public short ShowWindow;
            public short Reserved2Size;
            public IntPtr Reserved2;
            public IntPtr StandardInput;
            public IntPtr StandardOutput;
            public IntPtr StandardError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInformation {
            public IntPtr Process;
            public IntPtr Thread;
            public uint ProcessId;
            public uint ThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct FileTime {
            public uint Low;
            public uint High;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct BasicLimitInformation {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public IntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IoCounters {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ExtendedLimitInformation {
            public BasicLimitInformation BasicLimitInformation;
            public IoCounters IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern JobHandle CreateJobObject(IntPtr attributes, string name);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcess(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref StartupInformation startupInformation,
            out ProcessInformation processInformation);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(JobHandle job, int informationClass, ref ExtendedLimitInformation information, uint length);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(JobHandle job, IntPtr process);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetProcessTimes(IntPtr process, out FileTime creation, out FileTime exit, out FileTime kernel, out FileTime user);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(JobHandle job, uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true, EntryPoint = "CloseHandle")]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool CloseNativeHandle(IntPtr handle);

        private static void ThrowLastError(string operation) {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), operation);
        }

        private static void SetKillOnClose(JobHandle job, bool enabled) {
            var information = new ExtendedLimitInformation();
            information.BasicLimitInformation.LimitFlags = enabled ? KillOnJobClose : 0;
            uint size = (uint)Marshal.SizeOf(typeof(ExtendedLimitInformation));
            if (!SetInformationJobObject(job, 9, ref information, size)) {
                ThrowLastError("SetInformationJobObject failed");
            }
        }

        public static JobHandle CreateKillOnClose() {
            JobHandle job = CreateJobObject(IntPtr.Zero, null);
            if (job == null || job.IsInvalid) ThrowLastError("CreateJobObject failed");
            try {
                SetKillOnClose(job, true);
                return job;
            } catch {
                job.Dispose();
                throw;
            }
        }

        private static long ToDateTimeTicks(FileTime value) {
            long fileTime = ((long)value.High << 32) | value.Low;
            return DateTime.FromFileTimeUtc(fileTime).Ticks;
        }

        public static StartedProcessIdentity StartSuspendedInJob(JobHandle job, string executable, string commandLine, string currentDirectory) {
            var startup = new StartupInformation();
            startup.Size = (uint)Marshal.SizeOf(typeof(StartupInformation));
            startup.Flags = StartfUseShowWindow;
            startup.ShowWindow = 0;
            ProcessInformation process;
            if (!CreateProcess(executable, new StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero, false,
                CreateSuspended | CreateNoWindow, IntPtr.Zero, currentDirectory, ref startup, out process)) {
                ThrowLastError("CreateProcessW failed");
            }
            try {
                FileTime creation;
                FileTime exit;
                FileTime kernel;
                FileTime user;
                if (!GetProcessTimes(process.Process, out creation, out exit, out kernel, out user)) {
                    int error = Marshal.GetLastWin32Error();
                    TerminateProcess(process.Process, 1);
                    throw new System.ComponentModel.Win32Exception(error, "GetProcessTimes failed");
                }
                if (!AssignProcessToJobObject(job, process.Process)) {
                    int error = Marshal.GetLastWin32Error();
                    TerminateProcess(process.Process, 1);
                    throw new System.ComponentModel.Win32Exception(error, "AssignProcessToJobObject failed");
                }
                if (ResumeThread(process.Thread) == UInt32.MaxValue) {
                    int error = Marshal.GetLastWin32Error();
                    TerminateProcess(process.Process, 1);
                    throw new System.ComponentModel.Win32Exception(error, "ResumeThread failed");
                }
                return new StartedProcessIdentity((int)process.ProcessId, ToDateTimeTicks(creation));
            } finally {
                CloseNativeHandle(process.Thread);
                CloseNativeHandle(process.Process);
            }
        }

        public static void Terminate(JobHandle job) {
            if (!job.IsClosed && !job.IsInvalid && !TerminateJobObject(job, 1)) {
                ThrowLastError("TerminateJobObject failed");
            }
        }

        public static void ReleaseWithoutTerminate(JobHandle job) {
            if (!job.IsClosed && !job.IsInvalid) SetKillOnClose(job, false);
        }
    }
}
'@
}

function ConvertTo-WindowsCommandLineArgument {
    param([AllowEmptyString()][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
        } elseif ($character -eq '"') {
            [void]$builder.Append(('\' * (($backslashes * 2) + 1)))
            [void]$builder.Append('"')
            $backslashes = 0
        } else {
            if ($backslashes -gt 0) { [void]$builder.Append(('\' * $backslashes)) }
            [void]$builder.Append($character)
            $backslashes = 0
        }
    }
    if ($backslashes -gt 0) { [void]$builder.Append(('\' * ($backslashes * 2))) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Get-CanonicalExecutablePath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [System.IO.Path]::GetFullPath((Get-Item -LiteralPath $Path -ErrorAction Stop).FullName).TrimEnd('\')
}

function New-ManagedProcessJob {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][guid]$SessionId)

    if ($PSVersionTable.PSVersion.Major -ge 6 -and -not $IsWindows) {
        throw 'Managed process Jobs are supported only on Windows.'
    }
    [pscustomobject]@{
        session_id = $SessionId.ToString('D')
        handle = [ParallelWorld.Runtime.ManagedProcessJobNative]::CreateKillOnClose()
        identities = [System.Collections.ArrayList]::new()
        closed = $false
    }
}

function Start-ManagedProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]$Job,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($Job.closed) { throw 'The managed process Job is already closed.' }
    $executable = Get-CanonicalExecutablePath $FilePath
    $working = [System.IO.Path]::GetFullPath((Get-Item -LiteralPath $WorkingDirectory -ErrorAction Stop).FullName)
    $serialized = @($ArgumentList | ForEach-Object { ConvertTo-WindowsCommandLineArgument ([string]$_) }) -join ' '
    $commandLine = (ConvertTo-WindowsCommandLineArgument $executable) + $(if ($serialized) { ' ' + $serialized } else { '' })
    $nativeIdentity = [ParallelWorld.Runtime.ManagedProcessJobNative]::StartSuspendedInJob($Job.handle, $executable, $commandLine, $working)
    $identity = [pscustomobject]@{
        session_id = [string]$Job.session_id
        pid = [int]$nativeIdentity.ProcessId
        start_time_utc_ticks = [long]$nativeIdentity.StartTimeUtcTicks
        executable_path = [string]$executable
    }
    [void]$Job.identities.Add($identity)
    return $identity
}

function Get-ManagedProcessIdentityStatus {
    param([Parameter(Mandatory = $true)]$Identity)
    $process = Get-Process -Id ([int]$Identity.pid) -ErrorAction SilentlyContinue
    if ($null -eq $process) { return 'absent' }
    try {
        $startTicks = $process.StartTime.ToUniversalTime().Ticks
        $path = Get-CanonicalExecutablePath $process.Path
        if ([long]$Identity.start_time_utc_ticks -eq [long]$startTicks -and
            [string]::Equals([string]$Identity.executable_path, $path, [System.StringComparison]::OrdinalIgnoreCase)) {
            return 'matching'
        }
        return 'mismatch'
    } catch {
        return 'mismatch'
    }
}

function Stop-ManagedProcessJob {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]$Job,
        [ValidateRange(0, 300)][int]$GraceSeconds = 5
    )

    if ($Job.closed) { return }
    try {
        $statuses = @($Job.identities | ForEach-Object {
            [pscustomobject]@{
                identity = $_
                status = $(if ($_.session_id -ne $Job.session_id) { 'mismatch' } else { Get-ManagedProcessIdentityStatus $_ })
            }
        })
        if (@($statuses | Where-Object { $_.status -eq 'mismatch' }).Count -gt 0) {
            [ParallelWorld.Runtime.ManagedProcessJobNative]::ReleaseWithoutTerminate($Job.handle)
            return
        }
        $matching = @($statuses | Where-Object { $_.status -eq 'matching' } | ForEach-Object { $_.identity })

        foreach ($identity in $matching) {
            try {
                $process = Get-Process -Id ([int]$identity.pid) -ErrorAction Stop
                [void]$process.CloseMainWindow()
            } catch { }
        }
        if ($GraceSeconds -gt 0 -and $matching.Count -gt 0) {
            $deadline = [DateTime]::UtcNow.AddSeconds($GraceSeconds)
            while ([DateTime]::UtcNow -lt $deadline) {
                $remaining = @($matching | Where-Object { (Get-ManagedProcessIdentityStatus $_) -eq 'matching' })
                if ($remaining.Count -eq 0) { break }
                Start-Sleep -Milliseconds 100
            }
        }
        [ParallelWorld.Runtime.ManagedProcessJobNative]::Terminate($Job.handle)
    } finally {
        $Job.handle.Dispose()
        $Job.closed = $true
        $Job.identities.Clear()
    }
}

Export-ModuleMember -Function New-ManagedProcessJob, Start-ManagedProcess, Stop-ManagedProcessJob
