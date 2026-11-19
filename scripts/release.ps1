#Requires -Version 7
<#
.SYNOPSIS
모눈 한 버전을 GitHub 릴리스로 냅니다: 설치 파일(Monun-<버전>-Setup.exe)을 Releases 페이지에 올립니다.

.DESCRIPTION
  1. 확인: 고친 내용이 없고 HEAD가 origin/main과 같은지, 새 버전인지, Inno Setup(ISCC)과 로그인한
     GitHub CLI(gh)가 있는지.
  2. 루트 Cargo.toml의 [workspace.package] version을 바꾸고 Cargo.lock을 맞춥니다.
  3. monun.exe를 릴리스로 빌드합니다. target\release의 모눈이 실행 중이면(Windows가 exe를 잠급니다)
     그 모눈은 그대로 두고 target\installer에서 따로 빌드합니다(처음 한 번은 오래 걸립니다).
     exe가 Visual C++ 런타임 DLL 없이 실행되는지 확인하고, packaging\monun.iss로
     dist\Monun-<버전>-Setup.exe를 만듭니다.
  4. "모눈 <버전>"으로 커밋하고 v<버전> 태그를 붙여, origin main과 태그를 한 번에 올립니다.
  5. 설치 파일과, 지난 릴리스 뒤 커밋 제목 목록을 적은 노트로 GitHub 릴리스를 만듭니다.
-BuildOnly는 3단계만 합니다(git과 GitHub는 그대로). 설치 파일을 시험해 볼 때 씁니다.

.EXAMPLE
pwsh scripts/release.ps1 -Version 1.0.1

.EXAMPLE
pwsh scripts/release.ps1 -BuildOnly
#>
param(
    # 낼 버전(x.y.z). -BuildOnly에서는 생략하면 Cargo.toml의 버전이고, 주면 설치 파일의 버전만 바뀝니다.
    [string]$Version,
    # 빌드와 설치 파일만 만들고, 버전·커밋·태그·푸시·릴리스는 하지 않습니다.
    [switch]$BuildOnly,
    # 릴리스 노트의 변경 목록을 이 커밋(또는 태그) 다음부터 적습니다. 기본은 origin에 있는 지난 v* 태그.
    [string]$Since
)

$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH" }

$cargoToml = Join-Path $root 'Cargo.toml'
$distDir = Join-Path $root 'dist'
$iss = Join-Path $root 'packaging\monun.iss'

function Stop-Release([string]$message) {
    Write-Host ''
    Write-Host $message -ForegroundColor Red
    exit 1
}

function Step([string]$message) {
    Write-Host ''
    Write-Host $message -ForegroundColor Cyan
}

# Runs a native command and stops the release when it fails.
function Invoke-Checked([string]$what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { Stop-Release "$what에 실패했습니다 (종료 코드 $LASTEXITCODE)." }
}

function Find-Iscc {
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $dirs = @("${env:ProgramFiles(x86)}\Inno Setup 6", "$env:ProgramFiles\Inno Setup 6", "$env:LOCALAPPDATA\Programs\Inno Setup 6")
    foreach ($key in @(
            'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1')) {
        $location = (Get-ItemProperty $key -ErrorAction SilentlyContinue).InstallLocation
        if ($location) { $dirs += $location }
    }
    foreach ($dir in $dirs) {
        $path = Join-Path $dir 'ISCC.exe'
        if (Test-Path $path) { return $path }
    }
    $null
}

function Find-Gh {
    $cmd = Get-Command gh.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    foreach ($path in @("$env:ProgramFiles\GitHub CLI\gh.exe", "$env:LOCALAPPDATA\Programs\GitHub CLI\gh.exe")) {
        if (Test-Path $path) { return $path }
    }
    $null
}

# dumpbin comes with the Visual Studio Build Tools that build the app.
function Find-Dumpbin {
    $cmd = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { return $null }
    & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -First 1
}

$versionPattern = '(?ms)(^\[workspace\.package\][^\[]*?^version\s*=\s*")([^"]+)(")'
$tomlText = [IO.File]::ReadAllText($cargoToml)
$found = [regex]::Match($tomlText, $versionPattern)
if (-not $found.Success) { Stop-Release 'Cargo.toml의 [workspace.package]에서 version을 찾지 못했습니다.' }
$current = $found.Groups[2].Value

if (-not $Version) {
    if (-not $BuildOnly) { Stop-Release '낼 버전을 주세요: pwsh scripts/release.ps1 -Version x.y.z' }
    $Version = $current
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Stop-Release "버전은 1.2.3처럼 숫자 세 개여야 합니다: '$Version'" }
$tag = "v$Version"

# --- 1. 확인 ---------------------------------------------------------------------------------
Step "모눈 $Version $(if ($BuildOnly) { '설치 파일 빌드(시험용)' } else { '릴리스' })"

$iscc = Find-Iscc
if (-not $iscc) { Stop-Release 'Inno Setup 6(6.7 이상)이 없습니다: winget install JRSoftware.InnoSetup' }
$dumpbin = Find-Dumpbin
if (-not $dumpbin) { Stop-Release 'dumpbin을 찾지 못했습니다 (Visual Studio Build Tools의 C++ 도구).' }

if (-not $BuildOnly) {
    if ([version]$Version -le [version]$current) { Stop-Release "새 버전은 지금 버전($current)보다 커야 합니다." }
    $gh = Find-Gh
    if (-not $gh) { Stop-Release 'GitHub CLI가 없습니다: winget install GitHub.cli 뒤 gh auth login' }
    & $gh auth status *> $null
    if ($LASTEXITCODE -ne 0) { Stop-Release 'GitHub CLI에 로그인되어 있지 않습니다: gh auth login' }
    $originUrl = git remote get-url origin
    if ($originUrl -notmatch 'github\.com[:/](?<repo>[^/]+/[^/]+?)(\.git)?$') { Stop-Release "origin이 GitHub 저장소가 아닙니다: $originUrl" }
    $repo = $Matches['repo']

    if (git status --porcelain --untracked-files=no) { Stop-Release '커밋하지 않은 변경이 있습니다. 커밋하거나 되돌린 뒤 다시 실행하세요.' }
    Invoke-Checked 'origin에서 받기' { git fetch --quiet origin main }
    if ((git rev-parse HEAD) -ne (git rev-parse origin/main)) {
        Stop-Release 'HEAD가 origin/main과 다릅니다. 릴리스는 origin main에 올라간 코드로만 만듭니다 (먼저 pull 또는 push).'
    }
    if (git tag --list $tag) { Stop-Release "태그 $tag가 이미 있습니다." }
    if (git ls-remote --tags origin "refs/tags/$tag") { Stop-Release "origin에 태그 $tag가 이미 있습니다." }
}

# --- 2. 버전 ---------------------------------------------------------------------------------
$bumped = $false
$committed = $false
try {
    if (-not $BuildOnly) {
        Step "버전 $current → $Version"
        $at = $found.Groups[2]
        $newText = $tomlText.Substring(0, $at.Index) + $Version + $tomlText.Substring($at.Index + $at.Length)
        [IO.File]::WriteAllText($cargoToml, $newText, $utf8)
        $bumped = $true
        # Rewrites the workspace's own packages in Cargo.lock to the new version, nothing else.
        Invoke-Checked 'Cargo.lock 갱신' { cargo update --workspace --quiet }
    }

    # --- 3. 빌드와 설치 파일 ---------------------------------------------------------------------
    # Windows locks a running exe, so a 모눈 started from target\release (the desktop shortcut's)
    # would fail the build. Then the release gets a build folder of its own and that 모눈 runs on.
    $buildDir = Join-Path $root 'target'
    $usual = [IO.Path]::GetFullPath((Join-Path $buildDir 'release\monun.exe'))
    if (Get-Process monun -ErrorAction SilentlyContinue | Where-Object { $_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq $usual }) {
        $buildDir = Join-Path $root 'target\installer'
        Step '빌드하는 중… (target\release의 모눈이 실행 중이라 target\installer에서 따로 빌드합니다. 처음에는 10분 넘게 걸립니다)'
    } else {
        Step '빌드하는 중…'
    }
    # build.rs: without the icon and version resources the build fails instead of warning.
    $env:MONUN_REQUIRE_WINRES = '1'
    Invoke-Checked 'monun.exe 빌드' { cargo build --release --locked -p monun --target-dir $buildDir }
    $exe = Join-Path $buildDir 'release\monun.exe'

    # .cargo/config.toml links the C runtime statically; a DLL of it here would stop the app on
    # a PC without the Visual C++ Redistributable.
    $dependents = & $dumpbin /nologo /dependents $exe
    if ($LASTEXITCODE -ne 0) { Stop-Release 'dumpbin으로 exe의 DLL 목록을 읽지 못했습니다.' }
    $runtime = $dependents | Where-Object { $_ -match '^\s+(vcruntime|msvcp|api-ms-win-crt-|ucrtbase)' } | ForEach-Object { $_.Trim() }
    if ($runtime) { Stop-Release "monun.exe가 C 런타임 DLL을 씁니다: $($runtime -join ', ') (.cargo/config.toml의 +crt-static 확인)" }
    Write-Host 'C 런타임: 정적 링크 (VC++ 재배포 패키지 필요 없음)'

    # The licences of the crates built into the exe go beside it in the setup. xtask builds in
    # release: the antivirus blocks a dev build of serde's build script.
    $thirdParty = Join-Path $buildDir 'THIRD-PARTY-LICENSES.txt'
    Invoke-Checked '라이선스 목록 만들기' { cargo run --release --locked --quiet -p xtask --no-default-features --target-dir $buildDir -- third-party $thirdParty }

    Step '설치 파일을 만드는 중… (1분쯤)'
    New-Item -ItemType Directory -Force $distDir | Out-Null
    # Writing the icon into the setup file fails now and then ("EndUpdateResource failed (110)"):
    # the antivirus holds the new file for a moment. That error comes before the long compression,
    # so trying again costs little.
    for ($attempt = 1; ; $attempt++) {
        $isccOutput = & $iscc /Q "/DAppVersion=$Version" "/DSourceExe=$exe" "/DOutputDir=$distDir" "/DThirdParty=$thirdParty" $iss 2>&1
        if ($LASTEXITCODE -eq 0) { break }
        if ($attempt -lt 10 -and ($isccOutput -match 'EndUpdateResource')) {
            Write-Host "  아이콘을 쓰지 못해 다시 시도합니다 ($attempt/10)" -ForegroundColor Yellow
            continue
        }
        $isccOutput | ForEach-Object { Write-Host "  $_" }
        Stop-Release "설치 파일 만들기(ISCC)에 실패했습니다 (종료 코드 $LASTEXITCODE)."
    }
    $setup = Join-Path $distDir "Monun-$Version-Setup.exe"
    if (-not (Test-Path $setup)) { Stop-Release "설치 파일이 없습니다: $setup" }
    $sizeMb = [math]::Round((Get-Item $setup).Length / 1MB, 1)
    $sha256 = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLowerInvariant()
    Write-Host "설치 파일: $setup ($sizeMb MB)"
    Write-Host "SHA-256: $sha256"
    if ($BuildOnly) { exit 0 }

    # --- 4. 커밋, 태그, 푸시 ---------------------------------------------------------------------
    Step "커밋과 태그 $tag"
    $previous = git rev-parse HEAD
    Invoke-Checked '커밋' { git commit --quiet -m "모눈 $Version" -- Cargo.toml Cargo.lock }
    $committed = $true
    Invoke-Checked '태그' { git tag -a $tag -m "모눈 $Version" }
    # Both or neither: a main that moved since the fetch rejects the push, and nothing is left half done.
    git push --atomic origin HEAD:refs/heads/main "refs/tags/$tag"
    if ($LASTEXITCODE -ne 0) {
        git tag -d $tag | Out-Null
        git reset --quiet --keep $previous
        $committed = $false
        $bumped = $false
        Stop-Release 'origin에 올리지 못해 커밋과 태그를 되돌렸습니다. 그 사이 main이 바뀌었다면 pull한 뒤 다시 실행하세요.'
    }
    $bumped = $false

    # --- 5. GitHub 릴리스 -------------------------------------------------------------------------
    Step 'GitHub 릴리스를 만드는 중…'
    if (-not $Since) {
        # The newest v* tag on origin that this release contains (local tags from elsewhere don't count).
        $published = @(git ls-remote --tags --refs origin 'refs/tags/v*' | ForEach-Object { ($_ -split "`t")[1] -replace '^refs/tags/', '' })
        $Since = git for-each-ref --merged "$tag~1" --sort=-v:refname --format='%(refname:short)' 'refs/tags/v*' |
            Where-Object { $_ -ne $tag -and $published -contains $_ } | Select-Object -First 1
    }
    $lines = @(
        '## 설치'
        ''
        "아래 **Monun-$Version-Setup.exe**를 받아 실행합니다. 관리자 권한 없이 이 사용자에게만 설치되고, 이미 설치된 모눈은 이 버전으로 바뀝니다(설정은 그대로)."
        "서명하지 않은 설치 파일이라 'Windows의 PC 보호' 창이 뜨면 **추가 정보 → 실행**을 누릅니다."
        ''
        "SHA-256: ``$sha256``"
        ''
    )
    if ($Since) {
        # Named by the version it had ("1.0.0 이후"), whether -Since is a tag or a commit.
        $sinceToml = (git show "${Since}:Cargo.toml") -join "`n"
        $sinceVersion = [regex]::Match($sinceToml, $versionPattern)
        $sinceName = if ($sinceVersion.Success) { $sinceVersion.Groups[2].Value } else { $Since }
        $lines += "## 변경 사항 ($sinceName 이후)"
        $lines += ''
        $lines += @(git log --no-merges --format='- %s (%h)' "$Since..$tag~1")
    }
    $notes = Join-Path ([IO.Path]::GetTempPath()) "monun-$tag-notes.md"
    [IO.File]::WriteAllText($notes, ($lines -join "`n") + "`n", $utf8)
    & $gh release create $tag $setup --repo $repo --title "모눈 $Version" --notes-file $notes --verify-tag
    if ($LASTEXITCODE -ne 0) {
        Stop-Release "커밋과 태그는 올렸지만 릴리스를 만들지 못했습니다. 다시 하려면:`n  gh release create $tag `"$setup`" --repo $repo --title `"모눈 $Version`" --notes-file `"$notes`" --verify-tag"
    }
    Remove-Item $notes -ErrorAction SilentlyContinue
    Write-Host ''
    Write-Host "릴리스 완료: https://github.com/$repo/releases/tag/$tag" -ForegroundColor Green
}
finally {
    # A failure between the version change and the push puts Cargo.toml and Cargo.lock back.
    if ($bumped -and -not $committed) {
        git checkout --quiet -- Cargo.toml Cargo.lock
        Write-Host '버전 변경을 되돌렸습니다.' -ForegroundColor Yellow
    }
}
