# 모눈 개발 가이드

모눈을 직접 빌드하고, 고치고, 릴리스하는 방법입니다. 만든 배경과 설계 원칙은 [기획서](PLANNING.md)에 있습니다.

## 준비물

| 도구 | 용도 |
|---|---|
| [Rust](https://rustup.rs/) (rustup, stable) | 빌드 |
| Visual Studio Build Tools (C++ 데스크톱 개발) | Windows 링커 |
| [Inno Setup](https://jrsoftware.org/isinfo.php) 6.7 이상 | 설치 파일 만들기 (릴리스할 때만) |
| [GitHub CLI](https://cli.github.com/) | GitHub 릴리스 만들기 (릴리스할 때만) |

```powershell
winget install Rustlang.Rustup
winget install JRSoftware.InnoSetup
winget install GitHub.cli
```

## 빌드와 실행

```powershell
cargo build --release -p monun -p monun-cli   # target\release\monun.exe, monun-cli.exe
cargo run --release -p monun                  # 바로 실행
```

- exe는 C 런타임을 정적으로 링크하므로(`.cargo/config.toml`) Visual C++ 재배포 패키지가 필요 없습니다.
- 실행 중인 exe는 Windows가 잠그므로, 빌드 전에 모눈을 닫습니다.
- 릴리스 빌드를 보통 실행하면 모눈이 엑셀 · 문서 · 소스 파일의 '연결 프로그램'에 등록됩니다(현재 사용자만, 기본 앱은 바꾸지 않음, `apps/monun/src/file_types.rs`). 디버그 빌드와 `--control` 실행은 등록하지 않습니다.

### 소스로 쓰는 PC의 업데이트

저장소 폴더의 `update.cmd`는 `scripts/update.ps1`을 실행합니다. origin `main`을 빨리감기로 받아 릴리스 exe를 다시 빌드하고, 모눈이 켜져 있으면 닫을 때까지 기다립니다. 소스에서 빌드한 모눈은 아래의 앱 안 업데이트를 하지 않습니다.

### 앱 안 업데이트 (설치한 모눈)

NanoGit의 방식과 같습니다(`crates/ui-egui/src/update.rs`).

- 설치본(exe 옆에 설치 프로그램의 `unins000.exe`가 있는 것, `apps/monun/src/main.rs`의 `installed`)만 스스로 업데이트합니다. 디버그 빌드, 소스 빌드, `--control`·`MONUN_NO_PREFS` 실행은 확인하지 않습니다.
- 켠 뒤 15초, 그다음 6시간마다 `https://github.com/hulbo/monun/releases/latest/download/latest.json`을 읽습니다(옵션의 '새 버전 자동 확인'으로 끔). `latest.json`이 없는 릴리스(1.1.8까지)는 새 버전이 없는 것으로 봅니다.
- '지금 업데이트': 설치 파일을 `%TEMP%\monun-update`에 받으며 SHA-256을 확인합니다. 주소가 이 저장소의 릴리스(`https://github.com/hulbo/monun/releases/download/`)가 아니거나 SHA-256이 맞지 않으면 실행하지 않습니다. 그다음 끝내기와 같이 저장 질문을 하고, 모눈이 끝날 때(`on_exit`) 설치 파일을 `/SILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH=1`로 실행합니다. 설치 프로그램은 `/RELAUNCH=1`이면 설치 뒤 모눈을 다시 켭니다(`packaging/monun.iss`의 `Relaunch`).
- 끝나기 전 버전을 `%APPDATA%\Monun\updated-from.txt`에 적어 두고, 새 버전이 켜질 때 한 번 읽어 '업데이트했습니다'를 알립니다.
- 바탕화면의 '모눈 업데이트' 바로가기는 설치 프로그램이 만듭니다(`monun.exe --update`, 작업 `updateicon`). 묻지 않고 받아 설치합니다. 모눈이 이미 켜져 있으면 single instance가 `update`를 넘겨받아 그 모눈이 업데이트합니다.
- 서명 대신 HTTPS와 SHA-256으로 확인합니다. 설치 파일과 `latest.json`이 같은 GitHub 릴리스에 있으므로, 저장소 권한을 가진 사람만 바꿀 수 있습니다.

## 저장소 구조

Cargo 작업 공간이며, 아래 층이 위 층을 모르도록 나눠져 있습니다(`cargo xtask layers`로 검사).

```
core · numfmt · markdown · pptx   기본 값, 숫자 서식, Markdown · 코드 색, PowerPoint → HTML
        ↓
formula · functions               수식 파서, 워크시트 함수
        ↓
model · calc · xlsx               통합 문서 모델, 계산, XLSX · CSV 읽기/쓰기
        ↓
chart                             차트 그리기
        ↓
engine                            세션 · 문서 · 명령(모든 사용자 동작), 파일 입출력
        ↓
ui-egui · mcp                     화면(egui), AI 에이전트용 MCP 서버
        ↓
apps/monun · monun-cli · monun-web   데스크톱 앱, 명령줄 도구, 웹 빌드
```

| 위치 | 내용 |
|---|---|
| `crates/functions/src/*.rs` | 워크시트 함수(`f!(...)` 행과 테스트) |
| `crates/engine/src/cmd/*` | 사용자가 하는 모든 동작(명령). 메뉴 · 단축키 · 매크로 · CLI · MCP가 같은 명령을 씁니다 |
| `crates/ui-egui/src/` | 리본, 그리드, 그룹 패널(`groups.rs`, `group_folders.rs`), 프로젝트(`projects.rs`), 문서 화면(`doc_view/`) |
| `crates/ui-egui/src/{pdf,word,hwp,api}_view/` | PDF · Word · 한글 · API 요청 탭 |
| `crates/ui-egui/src/i18n/ko.rs` | 한국어 번역 |
| `crates/markdown/src/code/` | 코드 색칠과 소스 파일 표(`source.rs`) |
| `packaging/monun.iss` | 설치 파일(Inno Setup) |
| `scripts/` | 업데이트 · 릴리스 스크립트 |
| `xtask/` | 검사 도구(계층, 자산, 번역, 제3자 라이선스) |

파일 종류마다 엔진은 문서 탭 하나로만 알고, 화면이 전용 라이브러리로 그립니다.

| 종류 | 화면 쪽 | 라이브러리 |
|---|---|---|
| PDF | `pdf_view` | [PdfCraft](https://github.com/storytold/pdfcraft) 렌더러(고정 커밋) |
| Word | `word_view` | [WordCraft](https://github.com/storytold/wordcraft) 엔진(고정 커밋) |
| 한글 | `hwp_view` | [rhwp](https://github.com/edwardkim/rhwp) 0.9.0 |
| PowerPoint | `slides_view` | 자체 `crates/pptx` → HTML, WebView2로 표시 |
| 웹 페이지 | `web_view` | `wry`(WebView2) |
| API 요청 | `api_view` | `ureq` |

## 검사와 테스트

커밋하기 전에 다음을 통과시킵니다.

```powershell
cargo fmt --all
cargo clippy --workspace --lib --bins -- -D warnings
cargo xtask layers        # 층 구조
cargo xtask assets        # 모든 그림 · 아이콘 파일이 ATTRIBUTION.md에 있는지
cargo xtask i18n-report   # 번역 누락
cargo test -p monun-ui-egui   # 고친 크레이트마다
```

- `monun-ui-egui`의 통합 테스트(`crates/ui-egui/tests/it/`)는 egui_kittest로 화면을 실제로 그리고 누릅니다. 테스트 화면에는 기본 글꼴만 있어 한글을 그리지 못하므로, 화면 테스트의 문자열은 영어로 씁니다.
- 테스트 · 예제 · 문서 · 커밋 메시지에는 **지어낸 이름**만 씁니다. 실제 회사 · 고객 · 업무 이름이나 경로를 넣지 않습니다.

## 앱을 명령으로 다루기

```powershell
$env:MONUN_NO_PREFS = 1
target\release\monun.exe --control 7979
```

```powershell
monun-cli send 7979 engine.execute '{"command":"file.new"}'
```

```powershell
monun-cli send 7979 ui.screenshot '{"path":"shot.png"}'
```

- `MONUN_NO_PREFS=1`이면 사용자 설정(`%APPDATA%\Monun`)을 읽지도 쓰지도 않고, `--control` 실행은 이미 켜진 모눈에 파일을 넘기지 않습니다.
- 쓸 수 있는 메서드는 `crates/ui-egui/src/control.rs`에 있습니다(`ui.click`, `ui.text`, `ui.key`, `ui.dialog`, `ui.screenshot`, `engine.execute` 등).
- 화면 없이: `monun-cli eval '=NUMBERSTRING(1500000,1)'`, `monun-cli run --cmd 'cell.set={"cell":"A1","input":"1"}' --out book.xlsx`.

## 지키는 규칙

- **멈추지 않기:** 제품 코드에 `unwrap()`, `expect()`, `panic!`, 입력에서 온 값으로 인덱싱, `unsafe`를 쓰지 않습니다. 오류는 `Result`로 돌려줍니다.
- **남의 자산 넣지 않기:** Microsoft 등 다른 회사의 아이콘 · 그림 · 글꼴 · 서식 파일을 넣지 않습니다. 글꼴 파일은 커밋하지 않고 시스템 글꼴을 씁니다. 모든 자산 파일은 `ATTRIBUTION.md`에 출처를 적습니다.
- **문서 보안(DRM):** DRM 파일을 풀거나 다른 프로그램인 척하지 않습니다.
- **새 문구**는 `lang.tr("...")`로 감싸고 `ko.rs`에 번역을 넣습니다.

## 릴리스 만들기

origin `main`에 올라간 깨끗한 상태에서 실행합니다.

```powershell
pwsh scripts/release.ps1 -Version 1.2.0
```

1. 루트 `Cargo.toml`의 버전을 올립니다.
2. 릴리스 빌드를 하고 `cargo xtask third-party`로 `THIRD-PARTY-LICENSES.txt`를 만듭니다.
3. Inno Setup으로 `dist\Monun-1.2.0-Setup.exe`를 만듭니다(백신이 아이콘 쓰기를 막으면 다시 시도합니다).
4. "모눈 1.2.0" 커밋과 `v1.2.0` 태그를 main에 올리고, 지난 릴리스 뒤의 커밋 목록으로 GitHub 릴리스를 만듭니다.
5. 릴리스에 설치 파일과 함께 `latest.json`(버전, 설치 파일 주소, SHA-256, 변경 목록, 날짜)을 올립니다. 설치한 모눈은 이것을 보고 업데이트합니다.

- `-BuildOnly`는 설치 파일만 만들고 버전 · git · GitHub는 건드리지 않습니다.
- `target\release`의 모눈이 실행 중이면 `target\installer`에서 따로 빌드합니다.
- 설치 파일은 사용자 단위로 `%LOCALAPPDATA%\Programs\Monun`에 설치되고, 라이선스 파일(`LICENSE-*`, `NOTICE`, `ATTRIBUTION.md`, `THIRD-PARTY-LICENSES.txt`)을 함께 넣습니다. 제거할 때는 자기가 등록한 연결 프로그램 항목만 지우고 `%APPDATA%\Monun`은 남깁니다.

## 디스크 관리

빌드 결과물이 빨리 커집니다(`target\debug\incremental`이 수십 GB가 되기도 합니다). 커밋한 뒤에는 `target\debug\incremental`을 지우고, 쓰지 않는 작업 폴더의 `target`도 지웁니다.
