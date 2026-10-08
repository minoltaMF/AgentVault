# Read-only Windows system context. This is not a per-thread ETW scheduler trace.
param(
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][string]$StopFile,
    [int]$DurationSeconds = 1800
)
$ErrorActionPreference = "Stop"
if ($DurationSeconds -le 0) { throw "DurationSeconds must be positive" }
$stream = [IO.File]::Open($Output, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$watch = [Diagnostics.Stopwatch]::StartNew()
try {
    while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds -and -not (Test-Path -LiteralPath $StopFile)) {
        $sample = @{ started_at = [DateTime]::UtcNow.ToString("o") }
        try {
            $sample.scheduler = Get-CimInstance Win32_PerfFormattedData_PerfOS_System |
                Select-Object ContextSwitchesPersec,ProcessorQueueLength
            $sample.disk = @(Get-CimInstance Win32_PerfRawData_PerfDisk_PhysicalDisk |
                Where-Object Name -eq "_Total" |
                Select-Object AvgDisksecPerRead,AvgDisksecPerRead_Base,Frequency_PerfTime,
                    DiskReadsPersec,DiskReadBytesPersec,Timestamp_PerfTime,CurrentDiskQueueLength)
        } catch { $sample.error = $_.Exception.Message }
        $sample.finished_at = [DateTime]::UtcNow.ToString("o")
        $writer.WriteLine(($sample | ConvertTo-Json -Depth 5 -Compress))
        $writer.Flush()
        Start-Sleep -Milliseconds 1000
    }
} finally { $writer.Dispose() }
