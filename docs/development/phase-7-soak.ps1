param(
    [Parameter(Mandatory = $true)]
    [string] $VideoPath,
    [Parameter(Mandatory = $true)]
    [string] $OutputPath,
    [int] $DurationMinutes = 15,
    [int] $SampleIntervalSeconds = 60,
    [int] $PauseResumeAtSeconds = -1,
    [int] $PauseResumeDurationSeconds = 10,
    [ValidateSet('debug', 'release')]
    [string] $BuildProfile = 'debug'
)

$ErrorActionPreference = 'Stop'
$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$hostPath = Join-Path $workspaceRoot "target\x86_64-pc-windows-msvc\$BuildProfile\wallpaper-host.exe"
$startupImage = Join-Path $workspaceRoot 'crates\media\tests\fixtures\pixels.png'
$videoPath = (Resolve-Path $VideoPath).Path
$outputPath = [System.IO.Path]::GetFullPath($OutputPath)

if (Test-Path -LiteralPath $outputPath) {
    throw "Output file already exists; choose a new path to preserve prior samples: $outputPath"
}

if ($env:OS -ne 'Windows_NT') {
    throw 'The Phase 7 host soak requires Windows.'
}
if (-not (Test-Path -LiteralPath $hostPath -PathType Leaf)) {
    throw "Build wallpaper-host for x86_64-pc-windows-msvc first: $hostPath"
}
if ($DurationMinutes -lt 1 -or $SampleIntervalSeconds -lt 1 -or
    $PauseResumeDurationSeconds -lt 1) {
    throw 'Duration, sample interval, and pause duration must be positive.'
}
if ($PauseResumeAtSeconds -lt -1 -or
    $PauseResumeAtSeconds -ge ($DurationMinutes * 60)) {
    throw 'Pause/resume time must be -1 (disabled) or fall within the soak duration.'
}
if ($PauseResumeAtSeconds -ge 0 -and
    ($PauseResumeAtSeconds % $SampleIntervalSeconds) -ne 0) {
    throw 'Pause/resume time must align with the sampling interval.'
}

$existingHosts = Get-Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ProcessName -eq 'wallpaper-host' }
if ($existingHosts) {
    throw 'A wallpaper-host process is already running; stop the test rather than attaching to another host.'
}

$runId = [guid]::NewGuid().ToString('N')
$dataRoot = Join-Path $env:TEMP "wallpaper-lite-phase7-$runId"
$appData = Join-Path $dataRoot 'appdata'
$localAppData = Join-Path $dataRoot 'localappdata'
New-Item -ItemType Directory -Path $appData, $localAppData, (Split-Path -Parent $outputPath) -Force | Out-Null

$process = $null
$script:requestId = 0L
$script:hostProcessId = 0
$csvStream = $null
$csvWriter = $null

if (-not ('Phase7PipeNative' -as [type])) {
    Add-Type -TypeDefinition @"
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class Phase7PipeNative {
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetNamedPipeServerProcessId(
        SafePipeHandle pipe, out uint serverProcessId);
}
"@
}

function Get-RemainingTimeoutMilliseconds {
    param([DateTime] $Deadline)

    $remaining = $Deadline - [DateTime]::UtcNow
    if ($remaining.TotalMilliseconds -le 0) {
        throw 'The host IPC transaction exceeded its 10-second deadline.'
    }
    return [Math]::Max(1, [int] [Math]::Ceiling($remaining.TotalMilliseconds))
}

function Wait-PipeTask {
    param(
        [System.Threading.Tasks.Task] $Task,
        [DateTime] $Deadline,
        [System.Threading.CancellationTokenSource] $CancellationSource
    )

    $timeoutMilliseconds = Get-RemainingTimeoutMilliseconds $Deadline
    if (-not $Task.Wait($timeoutMilliseconds)) {
        $CancellationSource.Cancel()
        throw [System.TimeoutException]::new('The host IPC transaction exceeded its 10-second deadline.')
    }
    return $Task.GetAwaiter().GetResult()
}

function Read-ExactBytes {
    param(
        [System.IO.Stream] $Stream,
        [byte[]] $Buffer,
        [int] $Count,
        [DateTime] $Deadline,
        [System.Threading.CancellationTokenSource] $CancellationSource
    )
    $offset = 0
    while ($offset -lt $Count) {
        $readTask = $Stream.ReadAsync(
            $Buffer, $offset, $Count - $offset,
            $CancellationSource.Token
        )
        $read = Wait-PipeTask -Task $readTask -Deadline $Deadline -CancellationSource $CancellationSource
        if ($read -le 0) {
            throw 'The host closed the metrics pipe before completing a response.'
        }
        $offset += $read
    }
}

function Invoke-HostCommand {
    param([object] $Command)

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $script:requestId++
    $request = @{
        version = 12
        request_id = $script:requestId
        command = $Command
    }
    $payload = [System.Text.Encoding]::UTF8.GetBytes(
        (ConvertTo-Json -InputObject $request -Depth 16 -Compress)
    )
    $lengthPrefix = [BitConverter]::GetBytes([int] $payload.Length)
    $pipe = [System.IO.Pipes.NamedPipeClientStream]::new(
        '.', 'wallpaper-lite', [System.IO.Pipes.PipeDirection]::InOut,
        [System.IO.Pipes.PipeOptions]::Asynchronous
    )
    $cancellationSource = $null
    try {
        $connectTimeoutMilliseconds = Get-RemainingTimeoutMilliseconds $deadline
        $connectTimeoutMilliseconds = [Math]::Min(5000, $connectTimeoutMilliseconds)
        $pipe.Connect($connectTimeoutMilliseconds)
        [uint32] $serverProcessId = 0
        $serverFound = [Phase7PipeNative]::GetNamedPipeServerProcessId(
            $pipe.SafePipeHandle, [ref] $serverProcessId
        )
        if (-not $serverFound) {
            $nativeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "Could not identify the named-pipe server process (Win32 error $nativeError)."
        }
        if ($serverProcessId -ne [uint32] $script:hostProcessId) {
            throw "Connected to wallpaper-host PID $serverProcessId, expected $($script:hostProcessId)."
        }

        $cancellationSource = [System.Threading.CancellationTokenSource]::new()
        $ioTimeoutMilliseconds = Get-RemainingTimeoutMilliseconds $deadline
        $cancellationSource.CancelAfter($ioTimeoutMilliseconds)

        $prefixTask = $pipe.WriteAsync(
            $lengthPrefix, 0, $lengthPrefix.Length,
            $cancellationSource.Token
        )
        $null = Wait-PipeTask -Task $prefixTask -Deadline $deadline -CancellationSource $cancellationSource
        $payloadTask = $pipe.WriteAsync(
            $payload, 0, $payload.Length,
            $cancellationSource.Token
        )
        $null = Wait-PipeTask -Task $payloadTask -Deadline $deadline -CancellationSource $cancellationSource

        $responsePrefix = New-Object byte[] 4
        Read-ExactBytes $pipe $responsePrefix 4 $deadline $cancellationSource
        $responseLength = [BitConverter]::ToInt32($responsePrefix, 0)
        if ($responseLength -lt 1 -or $responseLength -gt 16384) {
            throw "The host returned an invalid IPC frame size: $responseLength"
        }
        $responseBytes = New-Object byte[] $responseLength
        Read-ExactBytes $pipe $responseBytes $responseLength $deadline $cancellationSource
        return [System.Text.Encoding]::UTF8.GetString($responseBytes) | ConvertFrom-Json
    }
    finally {
        if ($cancellationSource) {
            $cancellationSource.Dispose()
        }
        $pipe.Dispose()
    }
}

function Get-PlaybackState {
    $response = Invoke-HostCommand 'get_playback_state'
    if ($response.result.error) {
        throw "GetPlaybackState failed: $($response.result.error.code)"
    }
    if (-not $response.result.playback_state) {
        throw 'The host returned an unexpected GetPlaybackState response.'
    }
    return $response.result.playback_state
}

function Get-MetricSnapshot {
    $response = Invoke-HostCommand 'get_performance_metrics'
    if (-not $response.result.performance_metrics) {
        throw 'The host returned an unexpected GetPerformanceMetrics response.'
    }
    return $response.result.performance_metrics
}

try {
    $csvStream = [System.IO.File]::Open(
        $outputPath,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::Read
    )
    $csvWriter = [System.IO.StreamWriter]::new(
        $csvStream, [System.Text.UTF8Encoding]::new($false)
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $hostPath
    $startInfo.WorkingDirectory = $workspaceRoot
    $startInfo.Arguments = '--image "' + $startupImage + '"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.EnvironmentVariables['APPDATA'] = $appData
    $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $localAppData
    $process = [System.Diagnostics.Process]::Start($startInfo)
    $script:hostProcessId = $process.Id

    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $lastStartupError = $null
    do {
        try {
            $null = Invoke-HostCommand 'ping'
            break
        }
        catch {
            $lastStartupError = $_.Exception.Message
            if ($lastStartupError -like 'Connected to wallpaper-host PID*' -or
                $lastStartupError -like 'Could not identify the named-pipe server process*') {
                throw
            }
            if ($process.HasExited) {
                throw "wallpaper-host exited during startup with code $($process.ExitCode)."
            }
            if ([DateTime]::UtcNow -ge $deadline) {
                throw "The host IPC pipe did not become ready within 30 seconds. Last error: $lastStartupError"
            }
            Start-Sleep -Milliseconds 250
        }
    } while ($true)

    $loadResponse = Invoke-HostCommand @{
        load_video = @{
            wallpaper_id = 111
            path = $videoPath
        }
    }
    if ($loadResponse.result.error) {
        throw "Video load request failed: $($loadResponse.result.error.code)"
    }
    $loadDeadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $state = Get-PlaybackState
        if (-not $state.loading_video) {
            break
        }
        if ([DateTime]::UtcNow -ge $loadDeadline) {
            throw 'The video source did not finish loading within 30 seconds.'
        }
        Start-Sleep -Milliseconds 250
    } while ($true)
    if ($state.last_error) {
        throw "Video load failed: $($state.last_error)"
    }

    $playResponse = Invoke-HostCommand 'play'
    if ($playResponse.result.error) {
        throw "Play failed: $($playResponse.result.error.code)"
    }
    $state = Get-PlaybackState
    if ($state.state -ne 'playing') {
        throw "Playback did not enter Playing state: $($state.state)"
    }

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $duration = [TimeSpan]::FromMinutes($DurationMinutes)
    $samples = [System.Collections.Generic.List[object]]::new()
    $nextSampleAt = [TimeSpan]::Zero
    $headerWritten = $false
    $pauseResumeResult = $null

    while ($nextSampleAt -le $duration) {
        $remaining = $nextSampleAt - $watch.Elapsed
        if ($remaining.TotalMilliseconds -gt 0) {
            Start-Sleep -Milliseconds ([int] $remaining.TotalMilliseconds)
        }
        if ($watch.Elapsed -gt $duration.Add([TimeSpan]::FromSeconds(2)) -and
            $nextSampleAt -lt $duration) {
            # Do not report a normal completion without a terminal sample.
            # This can happen when a controlled pause extends past a sample slot.
            $nextSampleAt = $duration
        }
        if ($process.HasExited) {
            throw "wallpaper-host exited during the soak with code $($process.ExitCode)."
        }
        $state = Get-PlaybackState
        $metrics = Get-MetricSnapshot
        $videoTarget = $metrics.targets |
            Where-Object { $_.mediaKind -eq 'video' } |
            Select-Object -First 1
        $hostProcess = Get-Process -Id $process.Id
        $causes = $metrics.activePolicyCauses

        $sample = [pscustomobject]@{
            elapsed_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
            sampled_at_unix_ms = $metrics.sampledAtUnixMs
            playback_state = $state.state
            media_time_seconds = [math]::Round($state.media_time_100ns / 10000000, 3)
            fps_limit = $state.fps_limit
            last_error = $state.last_error
            display_id = if ($videoTarget) { $videoTarget.displayId } else { $null }
            source_fps = if ($videoTarget) { $videoTarget.sourceFps } else { $null }
            rendered_fps = if ($videoTarget) { $videoTarget.renderedFps } else { $null }
            frame_interval_ms = if ($videoTarget) { $videoTarget.frameIntervalMs } else { $null }
            dropped_frames_total = if ($videoTarget) { $videoTarget.droppedFramesTotal } else { $null }
            decode_latency_ms = if ($videoTarget) { $videoTarget.decodeLatencyMs } else { $null }
            present_latency_ms = if ($videoTarget) { $videoTarget.presentLatencyMs } else { $null }
            process_working_set_bytes = $metrics.processWorkingSetBytes
            process_private_bytes = $metrics.processPrivateBytes
            process_handles = $hostProcess.Handles
            process_threads = $hostProcess.Threads.Count
            gpu_local_usage_bytes = $metrics.gpuLocalUsageBytes
            gpu_local_budget_bytes = $metrics.gpuLocalBudgetBytes
            fullscreen = if ($causes) { $causes.fullscreen } else { $null }
            session_locked = if ($causes) { $causes.sessionLocked } else { $null }
            on_battery = if ($causes) { $causes.onBattery } else { $null }
            effective_action = if ($causes) { $causes.effectiveAction } else { $null }
        }
        if (-not $headerWritten) {
            $csvLines = $sample | ConvertTo-Csv -NoTypeInformation
            foreach ($csvLine in $csvLines) {
                $csvWriter.WriteLine($csvLine)
            }
            $headerWritten = $true
        }
        else {
            $csvLines = $sample | ConvertTo-Csv -NoTypeInformation |
                Select-Object -Skip 1
            foreach ($csvLine in $csvLines) {
                $csvWriter.WriteLine($csvLine)
            }
        }
        $csvWriter.Flush()
        $samples.Add($sample)

        if ($null -eq $pauseResumeResult -and $PauseResumeAtSeconds -ge 0 -and
            $watch.Elapsed.TotalSeconds -ge $PauseResumeAtSeconds) {
            $actualTriggerSeconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
            $pauseResponse = Invoke-HostCommand 'pause'
            if ($pauseResponse.result.error) {
                throw "Controlled pause failed: $($pauseResponse.result.error.code)"
            }
            $pauseDeadline = [DateTime]::UtcNow.AddSeconds(5)
            do {
                $pausedState = Get-PlaybackState
                if ($pausedState.state -eq 'paused') { break }
                if ([DateTime]::UtcNow -ge $pauseDeadline) {
                    throw 'Playback did not enter Paused state during controlled pause.'
                }
                Start-Sleep -Milliseconds 100
            } while ($true)

            Start-Sleep -Seconds $PauseResumeDurationSeconds
            $resumeResponse = Invoke-HostCommand 'play'
            if ($resumeResponse.result.error) {
                throw "Controlled resume failed: $($resumeResponse.result.error.code)"
            }
            $resumeDeadline = [DateTime]::UtcNow.AddSeconds(5)
            do {
                $resumedState = Get-PlaybackState
                if ($resumedState.state -eq 'playing') { break }
                if ([DateTime]::UtcNow -ge $resumeDeadline) {
                    throw 'Playback did not return to Playing state after controlled resume.'
                }
                Start-Sleep -Milliseconds 100
            } while ($true)
            $pauseResumeResult = [pscustomobject]@{
                requested_at_seconds = $PauseResumeAtSeconds
                actual_trigger_seconds = $actualTriggerSeconds
                pause_duration_seconds = $PauseResumeDurationSeconds
                paused_state = $pausedState.state
                resumed_state = $resumedState.state
                paused_media_time_seconds = [math]::Round($pausedState.media_time_100ns / 10000000, 3)
                resumed_media_time_seconds = [math]::Round($resumedState.media_time_100ns / 10000000, 3)
            }
        }

        if ($nextSampleAt -eq $duration) {
            break
        }
        $nextSampleAt = [TimeSpan]::FromSeconds([math]::Min(
            $duration.TotalSeconds,
            $nextSampleAt.TotalSeconds + $SampleIntervalSeconds
        ))
    }

    $watch.Stop()
    $logDirectory = Join-Path $localAppData 'kr.soyuo.wallpaper\logs'
    $logStatus = 'unavailable'
    $logFiles = @()
    if (Test-Path -LiteralPath $logDirectory -PathType Container) {
        $logFiles = @(Get-ChildItem -LiteralPath $logDirectory -Filter 'host*.log' -File)
        $logStatus = if ($logFiles.Count -gt 0) { 'available' } else { 'empty' }
    }
    $errorLines = @()
    $warningLines = @()
    $presentationFailureLines = @()
    foreach ($logFile in $logFiles) {
        $logLines = Get-Content -LiteralPath $logFile.FullName
        $errorLines += $logLines | Where-Object { $_ -match '\sERROR\s' }
        $warningLines += $logLines | Where-Object { $_ -match '\sWARN\s' }
        $presentationFailureLines += $logLines |
            Where-Object { $_ -match 'event="video_frame_presentation_failed"|event="video_frame_presentation_error_detail"' }
    }
    [pscustomobject]@{
        duration_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
        requested_duration_seconds = $duration.TotalSeconds
        completion_status = if ($samples.Count -gt 0 -and
            $samples[$samples.Count - 1].elapsed_seconds -ge $duration.TotalSeconds) {
            'completed'
        } else {
            'incomplete'
        }
        sample_count = $samples.Count
        first_sample = $samples[0]
        last_sample = $samples[$samples.Count - 1]
        process_error_log_lines = if ($logStatus -eq 'available') { $errorLines.Count } else { $null }
        process_warning_log_lines = if ($logStatus -eq 'available') { $warningLines.Count } else { $null }
        controlled_pause_resume = $pauseResumeResult
        log_status = $logStatus
        presentation_failure_log_lines = if ($logStatus -eq 'available') { $presentationFailureLines } else { $null }
        process_id = $process.Id
        csv_path = $outputPath
        log_directory = $logDirectory
    } | ConvertTo-Json -Depth 8
}
finally {
    if ($process -and -not $process.HasExited) {
        $process.Kill()
        $process.WaitForExit()
    }
    if ($csvWriter) {
        $csvWriter.Flush()
        $csvWriter.Dispose()
    }
    elseif ($csvStream) {
        $csvStream.Dispose()
    }
    if ($process) {
        $process.Dispose()
    }
}
