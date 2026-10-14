param(
    [string]$Binary = '',
    [string]$BaselineBinary = '',
    [ValidateRange(1, 1000000)][int]$Rounds = 10,
    [ValidateRange(1, 256)][int]$Jobs = 4,
    [ValidateRange(1, 100)][int]$Repeats = 3,
    [ValidateSet('serial', 'parallel', 'summary')][string[]]$IncludeModes = @('serial', 'parallel', 'summary'),
    [UInt64]$Seed = 42,
    [ValidatePattern('^[A-Za-z0-9_,\-]+$')][string]$Bots = 'idle-v1,easy-v1,normal-v1,hard-v1',
    [ValidatePattern('^[A-Za-z0-9_,\-]+$')][string]$Scenarios = 'duel-v1,skirmish-v1',
    [string]$OutRoot = ''
)

# Windows profiler: wall time includes report writing; compilation is excluded.
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
if (!$Binary) { $Binary = Join-Path $repoRoot 'target/release/ra-arena.exe' }
$Binary = (Resolve-Path -LiteralPath $Binary).Path
if (!$OutRoot) { $OutRoot = Join-Path $repoRoot ('arena-results/benchmark-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$OutRoot = [IO.Path]::GetFullPath($OutRoot)
New-Item -ItemType Directory -Path $OutRoot -Force | Out-Null

$modes = @()
if ($BaselineBinary) {
    $modes += @{ Name = 'before'; Binary = (Resolve-Path -LiteralPath $BaselineBinary).Path; Flags = '' }
}
if ($IncludeModes -contains 'serial') { $modes += @{ Name = 'serial'; Binary = $Binary; Flags = '--jobs 1' } }
if ($IncludeModes -contains 'parallel') { $modes += @{ Name = 'parallel'; Binary = $Binary; Flags = "--jobs $Jobs" } }
if ($IncludeModes -contains 'summary') { $modes += @{ Name = 'summary'; Binary = $Binary; Flags = "--jobs $Jobs --summary" } }
$measurements = @()
$reference = $null

for ($repeat = 1; $repeat -le $Repeats; $repeat++) {
    foreach ($mode in $modes) {
        $prefix = Join-Path $OutRoot ($mode.Name + '-' + $repeat)
        $reportPath = $prefix + '.json'
        $process = New-Object Diagnostics.Process
        $process.StartInfo = New-Object Diagnostics.ProcessStartInfo
        $process.StartInfo.FileName = $mode.Binary
        $process.StartInfo.Arguments = 'run --rounds {0} --seed {1} --out "{2}" {3} --bots {4} --scenarios {5}' -f $Rounds, $Seed, $reportPath, $mode.Flags, $Bots, $Scenarios
        $process.StartInfo.WorkingDirectory = $repoRoot
        $process.StartInfo.UseShellExecute = $false
        $process.StartInfo.CreateNoWindow = $true
        $process.StartInfo.RedirectStandardOutput = $true
        $process.StartInfo.RedirectStandardError = $true
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $process.Start() | Out-Null
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $peakWorkingSet = 0L
        $peakPrivateBytes = 0L
        $cpuSeconds = 0.0
        while (!$process.HasExited) {
            try {
                $process.Refresh()
                $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.PeakWorkingSet64)
                $peakPrivateBytes = [Math]::Max($peakPrivateBytes, $process.PrivateMemorySize64)
                $cpuSeconds = $process.TotalProcessorTime.TotalSeconds
            }
            catch { } # Process may exit between the state check and sampling.
            Start-Sleep -Milliseconds 20
        }
        $process.WaitForExit()
        $timer.Stop()
        try { $cpuSeconds = $process.TotalProcessorTime.TotalSeconds } catch { }
        [IO.File]::WriteAllText($prefix + '.stdout', $stdout.GetAwaiter().GetResult())
        [IO.File]::WriteAllText($prefix + '.stderr', $stderr.GetAwaiter().GetResult())
        if ($process.ExitCode -ne 0) { throw "Benchmark failed: $($mode.Name); see $prefix.stderr" }
        $process.Dispose()

        # Ranking and scenario scores must agree across every mode and repeat.
        $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        $scores = @{ Overall = $report.leaderboard; Scenarios = $report.by_scenario } | ConvertTo-Json -Depth 20 -Compress
        if ($null -eq $reference) { $reference = $scores }
        elseif ($scores -cne $reference) { throw "Ranking mismatch: $reportPath" }
        $games = if ($report.summary) { $report.summary.games } else { $report.matches.Count }
        $ticks = if ($report.summary) { $report.summary.ticks } else { ($report.matches | Measure-Object -Property ticks -Sum).Sum }
        $measurements += [pscustomobject]@{
            Mode = $mode.Name
            Repeat = $repeat
            Games = $games
            Ticks = $ticks
            WallSeconds = [Math]::Round($timer.Elapsed.TotalSeconds, 4)
            CpuSeconds = [Math]::Round($cpuSeconds, 4)
            CpuMicrosecondsPerTick = if ($ticks -gt 0) { [Math]::Round($cpuSeconds * 1000000 / $ticks, 4) } else { 0 }
            GamesPerSecond = [Math]::Round($games / $timer.Elapsed.TotalSeconds, 2)
            PeakWorkingSetMiB = [Math]::Round($peakWorkingSet / 1MB, 2)
            SampledPeakPrivateMiB = [Math]::Round($peakPrivateBytes / 1MB, 2)
            ReportBytes = (Get-Item -LiteralPath $reportPath).Length
        }
    }
}
$measurements | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $OutRoot 'measurements.json') -Encoding UTF8
$measurements | Format-Table -AutoSize
Write-Output "Saved metrics and reports to $OutRoot"
