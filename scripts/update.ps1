# 모눈 업데이트: origin main의 최신 코드를 받아, 바탕화면 바로가기가 실행하는
# target\release\monun.exe(와 monun-cli.exe)를 다시 빌드합니다. update.cmd가 이 스크립트를 실행합니다.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Stop-WithMessage([string]$message) {
    Write-Host ''
    Write-Host $message -ForegroundColor Red
    Read-Host '엔터를 누르면 창이 닫힙니다' | Out-Null
    exit 1
}

Write-Host '모눈 업데이트' -ForegroundColor Cyan
Write-Host "폴더: $root"
Write-Host ''

# 실행 중인 exe는 Windows가 잠가서 빌드할 수 없습니다.
if (Get-Process monun -ErrorAction SilentlyContinue) {
    Write-Host '모눈이 실행 중입니다. 작업을 저장하고 모눈을 닫아 주세요. 닫히면 바로 이어서 진행합니다. (취소: Ctrl+C)' -ForegroundColor Yellow
    while (Get-Process monun -ErrorAction SilentlyContinue) {
        Start-Sleep -Seconds 2
    }
}

$branch = git branch --show-current
if ($branch -ne 'main') {
    Stop-WithMessage "이 폴더가 main이 아니라 '$branch' 브랜치에 있어 업데이트하지 않았습니다."
}

Write-Host '1/2 최신 코드를 받는 중…'
$before = git rev-parse HEAD
git fetch origin main
if ($LASTEXITCODE -ne 0) { Stop-WithMessage 'GitHub에서 코드를 받지 못했습니다. 인터넷 연결을 확인해 주세요.' }
git merge --ff-only origin/main
if ($LASTEXITCODE -ne 0) { Stop-WithMessage '이 폴더에 고친 내용이 있어 최신 코드를 합치지 못했습니다. (git status로 확인)' }
$changes = git log --oneline "$before..HEAD"
if ($changes) {
    Write-Host '새로 받은 변경:'
    $changes | ForEach-Object { Write-Host "  $_" }
} else {
    Write-Host '이미 최신 코드입니다.'
}

Write-Host ''
Write-Host '2/2 빌드하는 중… (처음이나 변경이 많으면 몇 분 걸립니다)'
cargo build --release -p monun -p monun-cli
if ($LASTEXITCODE -ne 0) { Stop-WithMessage '빌드에 실패했습니다. 위의 오류 메시지를 확인해 주세요.' }

Write-Host ''
Write-Host "업데이트 완료: $(git log -1 --format='%h %s')" -ForegroundColor Green
$answer = Read-Host '모눈을 지금 실행할까요? (Y/n)'
if ($answer -notmatch '^[nN]') {
    Start-Process (Join-Path $root 'target\release\monun.exe') -WorkingDirectory ([Environment]::GetFolderPath('MyDocuments'))
}
