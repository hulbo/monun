#Requires -Version 7
<#
.SYNOPSIS
Checks a Monun change before it is committed.

.DESCRIPTION
Phases (all three by default):
  check  cargo fmt --check, clippy -D warnings, cargo xtask layers and assets
  test   cargo test for each package the change touches; the known failures in CLAUDE.md pass
  app    release build, start the app with the control channel, send the -Setup commands,
         take a screenshot and read the app log
Logs and the screenshot go to target\verify. The output ends with a summary (also written to
target\verify\summary.txt); the exit code is 0 when everything passed, 1 when something failed
and 2 when the app phase was blocked because the user's own 모눈 is open.

.EXAMPLE
./.claude/skills/verify/verify.ps1 -Phase app -Setup 'cell.set={"cell":"A1","input":"한글 ABC 123"}'
#>
param(
    [ValidateSet('check', 'test', 'app')]
    [string[]]$Phase = @('check', 'test', 'app'),
    # Packages to test (monun-functions, monun-ui-egui, …); by default every package with
    # changes since -Base.
    [string[]]$Crates,
    # Changes are measured from this revision: HEAD for uncommitted work, HEAD~1 for the last commit.
    [string]$Base = 'HEAD',
    # Sent to the app before the screenshot, each `method` or `method={json}`: a control method
    # (ui.click, ui.text, ui.key, ui.dialog, …) or an engine command (cell.set, selection.set, …).
    [string[]]$Setup = @(),
    [int]$Port = 7979,
    # Leave the app running after the screenshot, to drive it further with `monun-cli send`.
    [switch]$KeepOpen
)

$ErrorActionPreference = 'Continue'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
Set-Location $root
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH" }
$logDir = Join-Path $root 'target\verify'
New-Item -ItemType Directory -Force $logDir | Out-Null

# Tests that fail on this machine whatever the change: they need an embedded font set (CLAUDE.md).
$knownFailures = @(
    'file_menu::save_a_copy_with_a_bare_name_asks_where_to_save_it',
    'interaction::column_autocomplete_folds_case_beyond_ascii',
    'interaction::language_switch_renders_the_ribbon_and_persists'
)
# monun-engine's unit tests use a unix-only API: on Windows only its integration tests build.
$testArgs = @{ 'monun-engine' = @('--test', '*') }

$summary = [System.Collections.Generic.List[string]]::new()
$script:failed = $false
$script:blocked = $false

function Add-Result([string]$step, [string]$status, [string]$detail = '') {
    if ($status -eq 'FAIL') { $script:failed = $true }
    if ($status -eq 'BLOCKED') { $script:blocked = $true }
    $summary.Add(('{0,-22} {1,-7} {2}' -f $step, $status, $detail).TrimEnd())
}

# Runs cargo with its output in target\verify\<name>.log.
function Invoke-Cargo([string]$name, [string[]]$arguments) {
    $log = Join-Path $logDir "$name.log"
    $start = Get-Date
    $lines = @(& cargo @arguments 2>&1 | ForEach-Object { "$_" })
    $code = $LASTEXITCODE
    $lines | Set-Content -Path $log -Encoding utf8
    [pscustomobject]@{ Code = $code; Lines = $lines; Log = $log; Seconds = [int]((Get-Date) - $start).TotalSeconds }
}

# The compiler's first messages, each with the place it points at.
function Get-Diagnostics([string[]]$lines, [int]$max = 8) {
    $noise = '^(error|warning): (could not compile|aborting due to|build failed|`[^`]+` \(.*\) generated \d+|\d+ warnings? emitted)'
    $found = @()
    for ($i = 0; $i -lt $lines.Count -and $found.Count -lt $max; $i++) {
        if ($lines[$i] -match '^(error|warning)(\[\w+\])?: ' -and $lines[$i] -notmatch $noise) {
            $at = if ($i + 1 -lt $lines.Count -and $lines[$i + 1] -match '-->\s*(.+)$') { " @ $($Matches[1].Trim())" } else { '' }
            $found += "    $($lines[$i])$at"
        }
    }
    $found
}

function Get-ChangedPackages {
    $files = @(git diff --name-only $Base) + @(git ls-files --others --exclude-standard)
    $dirs = [System.Collections.Generic.SortedSet[string]]::new()
    foreach ($f in $files) {
        if ($f -match '^(crates|apps)/([^/]+)/') { [void]$dirs.Add("$($Matches[1])/$($Matches[2])") }
        elseif ($f -match '^xtask/') { [void]$dirs.Add('xtask') }
        elseif ($f -in 'Cargo.toml', 'Cargo.lock') {
            Get-ChildItem crates, apps -Directory | ForEach-Object { [void]$dirs.Add("$($_.Parent.Name)/$($_.Name)") }
            [void]$dirs.Add('xtask')
        }
    }
    foreach ($d in $dirs) {
        $toml = Join-Path $root "$d/Cargo.toml"
        if (Test-Path $toml) {
            $m = Select-String -Path $toml -Pattern '^name\s*=\s*"([^"]+)"' | Select-Object -First 1
            if ($m) { $m.Matches[0].Groups[1].Value }
        }
    }
}

# The lines the app logged after byte `from` (the log is shared with the running app).
function Read-LogSince([string]$path, [long]$from) {
    if (-not (Test-Path $path)) { return @() }
    $fs = [System.IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')
    try {
        if ($from -gt $fs.Length) { $from = 0 }
        [void]$fs.Seek($from, 'Begin')
        @(([System.IO.StreamReader]::new($fs)).ReadToEnd() -split "`r?`n" | Where-Object { $_ })
    } finally { $fs.Dispose() }
}

if ('check' -in $Phase) {
    $r = Invoke-Cargo 'fmt' @('fmt', '--all', '--', '--check')
    if ($r.Code -eq 0) { Add-Result 'check: fmt' 'PASS' }
    else {
        $files = @($r.Lines | ForEach-Object { if ($_ -match '^Diff in (?:\\\\\?\\)?(.+?\.rs)') { $Matches[1].Replace("$root\", '') } } | Sort-Object -Unique)
        Add-Result 'check: fmt' 'FAIL' "run ``cargo fmt --all``: $($files -join ', ')"
    }

    $r = Invoke-Cargo 'clippy' @('clippy', '--workspace', '--lib', '--bins', '--', '-D', 'warnings')
    if ($r.Code -eq 0) { Add-Result 'check: clippy' 'PASS' "$($r.Seconds)s" }
    else { Add-Result 'check: clippy' 'FAIL' "log $($r.Log)"; Get-Diagnostics $r.Lines | ForEach-Object { $summary.Add($_) } }

    foreach ($task in 'layers', 'assets') {
        $r = Invoke-Cargo "xtask-$task" @('xtask', $task)
        if ($r.Code -eq 0) { Add-Result "check: xtask $task" 'PASS' }
        else {
            Add-Result "check: xtask $task" 'FAIL' "log $($r.Log)"
            $r.Lines | Where-Object { $_ -notmatch '^\s*(Compiling|Finished|Running|Blocking)\b' } | Select-Object -Last 6 | ForEach-Object { $summary.Add("    $_") }
        }
    }
}

if ('test' -in $Phase) {
    $packages = if ($Crates) { $Crates } else { @(Get-ChangedPackages) }
    if (-not $packages) { Add-Result 'test' 'SKIP' "no package has changes since $Base" }
    foreach ($p in $packages) {
        $extra = if ($testArgs.ContainsKey($p)) { $testArgs[$p] } else { @() }
        $r = Invoke-Cargo "test-$p" (@('test', '-p', $p, '--no-fail-fast') + $extra)
        $failures = @($r.Lines | Where-Object { $_ -match '^test (\S+) \.\.\. FAILED' } | ForEach-Object { ($_ -split ' ')[1] } | Sort-Object -Unique)
        $new = @($failures | Where-Object { $_ -notin $knownFailures })
        $known = $failures.Count - $new.Count
        $passed = 0
        $r.Lines | Where-Object { $_ -match '^test result: \w+\. (\d+) passed' } | ForEach-Object { $passed += [int]($_ -replace '^test result: \w+\. (\d+) passed.*$', '$1') }
        $brokenBuild = @($r.Lines | Where-Object { $_ -match '^error: could not compile' }).Count -gt 0
        if ($brokenBuild -or ($r.Code -ne 0 -and $failures.Count -eq 0)) {
            Add-Result "test: $p" 'FAIL' "does not build; log $($r.Log)"
            Get-Diagnostics $r.Lines | ForEach-Object { $summary.Add($_) }
        } elseif ($new) {
            Add-Result "test: $p" 'FAIL' "$($new.Count) failed, $passed passed; log $($r.Log)"
            $new | ForEach-Object { $summary.Add("    $_") }
        } else {
            $note = if ($known) { ", $known known failures" } else { '' }
            Add-Result "test: $p" 'PASS' "$passed passed$note, $($r.Seconds)s"
        }
    }
}

if ('app' -in $Phase) {
    $exe = Join-Path $root 'target\release\monun.exe'
    $cli = Join-Path $root 'target\release\monun-cli.exe'
    $running = { @(Get-CimInstance Win32_Process -Filter "Name = 'monun.exe'" | Where-Object { $_.ExecutablePath -eq $exe }) }
    # An instance this script left open (-KeepOpen) is ours to close; any other is the user's.
    foreach ($p in (& $running | Where-Object { $_.CommandLine -match "--control\s+$Port\b" })) {
        & $cli send $Port app.quit *> $null
        Wait-Process -Id $p.ProcessId -Timeout 15 -ErrorAction SilentlyContinue
    }
    if (& $running) {
        Add-Result 'app' 'BLOCKED' '모눈 is open from target\release and Windows locks the exe: ask the user to close it (never stop it yourself)'
    } else {
        $r = Invoke-Cargo 'build-release' @('build', '--release', '-p', 'monun', '-p', 'monun-cli')
        if ($r.Code -ne 0) {
            Add-Result 'app: release build' 'FAIL' "log $($r.Log)"
            Get-Diagnostics $r.Lines | ForEach-Object { $summary.Add($_) }
        } else {
            Add-Result 'app: release build' 'PASS' "$($r.Seconds)s"
            $appLog = Join-Path $env:APPDATA 'Monun\logs\monun.log'
            $logStart = if (Test-Path $appLog) { (Get-Item $appLog).Length } else { 0 }
            $proc = Start-Process -FilePath $exe -ArgumentList '--control', $Port -PassThru
            $ready = $false
            for ($i = 0; $i -lt 60 -and -not $ready -and -not $proc.HasExited; $i++) {
                Start-Sleep -Milliseconds 500
                & $cli send $Port ui.inspect *> $null
                $ready = $LASTEXITCODE -eq 0
            }
            if (-not $ready) {
                $how = if ($proc.HasExited) { "exited with code $($proc.ExitCode)" } else { 'did not answer on the control channel within 30 s' }
                Add-Result 'app: start' 'FAIL' "the app $how"
            } else {
                Add-Result 'app: start' 'PASS'
                foreach ($cmd in $Setup) {
                    $method, $json = $cmd -split '=', 2
                    $sendArgs = @('send', $Port, $method.Trim()) + @($json | Where-Object { $_ })
                    $out = @(& $cli @sendArgs 2>&1 | ForEach-Object { "$_" })
                    if ($LASTEXITCODE -ne 0) { Add-Result 'app: setup' 'FAIL' "$cmd -> $($out -join ' ')" }
                }
                Start-Sleep -Milliseconds 500
                $shot = Join-Path $logDir 'app.png'
                Remove-Item $shot -ErrorAction SilentlyContinue
                $out = @(& $cli send $Port ui.screenshot (@{ path = $shot } | ConvertTo-Json -Compress) 2>&1 | ForEach-Object { "$_" })
                if (Test-Path $shot) { Add-Result 'app: screenshot' 'PASS' $shot }
                else { Add-Result 'app: screenshot' 'FAIL' ($out -join ' ') }
            }
            if ($proc.HasExited -and $ready) { Add-Result 'app: still running' 'FAIL' "the app exited with code $($proc.ExitCode)" }
            if (-not $KeepOpen -and -not $proc.HasExited) {
                & $cli send $Port app.quit *> $null
                if (-not $proc.WaitForExit(15000)) { Add-Result 'app: quit' 'FAIL' 'the app did not close within 15 s' }
            } elseif ($KeepOpen -and -not $proc.HasExited) {
                $summary.Add("    left open: target\release\monun-cli.exe send $Port <method> '<json>', then app.quit")
            }
            $logged = Read-LogSince $appLog $logStart
            $panics = @($logged | Where-Object { $_ -match 'panicked' })
            $errors = @($logged | Where-Object { $_ -match '^\S+ ERROR\s' })
            if ($panics) { Add-Result 'app: log' 'FAIL' "$($panics.Count) panic lines"; $panics | Select-Object -First 5 | ForEach-Object { $summary.Add("    $_") } }
            elseif ($errors) { Add-Result 'app: log' 'NOTE' "$($errors.Count) ERROR lines (check whether the change caused them)"; $errors | Select-Object -First 5 | ForEach-Object { $summary.Add("    $_") } }
            else { Add-Result 'app: log' 'PASS' 'no errors or panics' }
        }
    }
}

$verdict = if ($script:failed) { 'FAIL' } elseif ($script:blocked) { 'BLOCKED' } else { 'PASS' }
$report = @("== verify: $verdict ==") + $summary + @("logs: $logDir")
$report | Set-Content -Path (Join-Path $logDir 'summary.txt') -Encoding utf8
''
$report
exit $(if ($script:failed) { 1 } elseif ($script:blocked) { 2 } else { 0 })
