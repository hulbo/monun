# 모눈 (Monun)

모눈종이처럼 칸이 나뉜 개인용 스프레드시트입니다. 순수 Rust로 만들었고, 엑셀(XLSX) 파일을 그대로
열고 저장합니다.

## 특징

- **엑셀 파일:** XLSX를 기본 형식으로 열고 저장합니다. CSV/TSV도 지원하고, 예전 `.xls`는 값과 수식을 읽습니다.
- **수식:** 500개 이상의 함수, 동적 배열, LET·LAMBDA, 구조적 표 참조를 지원합니다.
- **한글 금액:** `NUMBERSTRING`으로 숫자를 한글로 씁니다. 예: `="일금 "&NUMBERSTRING(A1,1)&"원정"`
- **한국어 화면:** 메뉴와 대화상자가 한국어로 나오고, 설치된 한글 글꼴을 한글 이름으로 고를 수 있습니다.
- **기본값:** 새 통합 문서는 굴림체, 기본 칸 너비 130px로 시작합니다. 바꾸려면
  `%APPDATA%\Monun\prefs.json`의 `defaultFont`, `defaultColumnWidth`를 고치면 됩니다.
- **화면:** 100% 배율이 윈도우 엑셀과 같은 크기이고, 글자를 얇고 선명하게 그립니다. 탭과 빠른 실행
  버튼이 한 줄에 있고, 리본은 Ctrl+F1이나 오른쪽 위 ▲ 버튼으로 숨길 수 있습니다.
- **한글 표시:** 한글과 영문이 같은 기준선에 놓이고, 굵은 글꼴 파일이 없는 글꼴(굴림체 등)도 굵게 보입니다.
- **차트·서식:** 조건부 서식, 표, 차트, 스파크라인, 피벗 테이블, 인쇄와 PDF 내보내기를 지원합니다.

## 빌드와 실행

Rust(rustup)와 Visual Studio Build Tools(C++)가 필요합니다.

```sh
cargo build --release -p monun        # target/release/monun.exe
cargo run --release -p monun          # 바로 실행
cargo run --release -p monun-cli -- eval '=NUMBERSTRING(1500000,1)'
```

설정과 로그는 `%APPDATA%\Monun`에 있습니다.

릴리스 빌드를 실행하면 모눈이 엑셀 파일(.xlsx, .xlsm, .xls 등)과 CSV의 '연결 프로그램' 목록에
등록됩니다(현재 사용자만, 관리자 권한 불필요). 엑셀 파일을 처음 열 때 모눈을 고르고 **항상**을 누르면
기본 앱이 되고 파일에 모눈 아이콘이 나옵니다. 설정 › 앱 › 기본 앱 › 모눈에서 한꺼번에 지정할 수도 있습니다.

## 업데이트

자동 업데이트는 없습니다. 이 저장소에서 변경 사항을 받아(`git pull`) 다시 빌드합니다.

## 라이선스

MIT 또는 Apache-2.0 중 하나를 선택해 따릅니다. 전문과 저작권 고지는 [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE)에 있고, 번들 자산의 출처는
[ATTRIBUTION.md](ATTRIBUTION.md)에 있습니다.
