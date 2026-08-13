# tmux Integration

방침은 **밀접한 통합, 내장 멀티플렉서 없음**입니다. cf. [0009](adr/0009-no-multiplexer.md)

v1은 수준 1(패스스루)입니다. 수준 2(컨트롤 모드)는 v2이며, `feed`가 PTY 없는 세션의 정규 입력이라는 점과 세션 생성 이원화가 그 문을 열어두는 장치입니다.

## Level 1 requirements

| 항목 | v1 요구 | 영향 |
|---|---|---|
| 타이틀 (OSC 0/2) | `set-titles` 반영. tmux가 내부 구조를 알려주는 주 통로 | 스냅샷 필드 |
| OSC 52 | `set-clipboard on` 경유 쓰기 | 이벤트 |
| allow-passthrough | tmux가 언래핑 → 일반 시퀀스로 수신. 미지 DCS는 파서가 조용히 무시 | VT 엔진 |
| 리사이즈 | 셀 단위 변화만 호출(코얼레싱), 창 크기 그리드 스냅 | Swift 정책 |
| 마우스 휠 | 리포팅 on → 마우스 코드 / 대체 화면 + alternate scroll → 방향키 / 그 외 → 스크롤백 | 입력 경로 3분기 |
| 포커스 (1004) | `focus-events on` → vim autoread 등 | 입력 경로 |
| 동기화 출력 (2026) | tmux 사용. 블록 중 wake 억제로 티어링 제거 | [C5](03-core.md#c5--wake-emission) |
| OSC 10/11 질의 | 내부 앱의 배경색 감지가 tmux를 관통 | 내부 응답 경로 |
| OSC 7 (작업 디렉터리) | tmux 기본 미전달. **문서화된 한계**. 비 tmux 환경을 위해 유지 | 스냅샷 필드 |
| 환경 | `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM`. tmux `terminal-features` 권장 설정은 사용자 문서 항목 | 스폰 파라미터 |

**OSC 8은 v1 범위가 아닙니다.** cf. [0006](adr/0006-no-osc8-in-v1.md) — 최신 tmux가 패스스루하더라도 우리가 셀→링크 연결을 받을 수 없습니다. URL은 ⌘ 스캔이 담당합니다.

## Derived decisions

- 스플릿은 v1에서 삭제합니다. 페인은 tmux의 일입니다. 세션 모델이 "창당 하나"로 단순해집니다.
- 스크롤백은 유지합니다. tmux 밖과 ssh 직결에서 필요합니다. tmux 안에서는 비게 됩니다.
    - "tmux에서 네이티브 스크롤이 안 된다"는 불만의 근본 해결은 v2 컨트롤 모드입니다.
- 타이틀은 유지하고 격상합니다. 자체 탭이 없는 환경에서 macOS 창을 구별하는 수단입니다.
