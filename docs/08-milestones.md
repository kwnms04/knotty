# Milestones

아키텍처가 강제하는 순서를 따릅니다. 하네스가 코어의 첫 소비자이고, 그다음은 "첫 픽셀까지 최단"입니다.
각 마일스톤은 기간이 아니라 **종료 기준**으로 정의합니다. 종료 기준에 없는 것은 하지 않습니다.

## M0 — Headless core

Swift 없이 knotty-core와 하네스만 만듭니다.

스펙과 티켓: [`.scratch/m0-headless-core/`](../.scratch/m0-headless-core/) (원래 #1과 #2~#9)

- detached 세션 + `feed` + 메일박스 + 평탄 POD 스냅샷 변환
- 헤더 생성, ABI 핸드셰이크, 불투명 핸들, 패닉 격리
- 하네스: 녹화 스트림 3종(vim, tmux, htop) 골든 통과
- **종료 기준**: 하네스 CI 녹색, 스냅샷 형식이 생성된 헤더와 바이트 단위로 일치

## M1 — PTY and threading

스펙과 티켓: [`.scratch/m1-pty-and-threading/`](../.scratch/m1-pty-and-threading/) (원래 #10과 #11~#29)

- I/O 스레드 ([C1](03-core.md#c1--thread-topology)), PTY 스폰, writer 큐 + 배압
- 자식 종료 처리 + 순서 불변식 ([C6](03-core.md#c6--child-lifecycle))
- 이벤트 큐 3종 (벨, 클립보드 쓰기, 자식 종료)
- **응답 위생 필터** ([C4](03-core.md#c4--listener-and-response-hygiene)) — PTY가 생겨야 검증 가능합니다
- `feed` 퍼저
- **VT 바인딩 파사드** ([0012](adr/0012-own-the-binding-layer.md)) — 마일스톤의 마지막
  작업입니다. 위 종료 기준 둘이 통과한 뒤에 시작하므로, 파사드가 무엇을 깨뜨리든
  그것이 파사드 탓임이 드러납니다
- **종료 기준**: B4·B5 벤치 통과, 퍼저 1시간 무크래시, 코어가 `libghostty-vt`를
  더는 의존하지 않음

## M2 — First pixel

스펙과 티켓: [`.scratch/m2-first-pixel/`](../.scratch/m2-first-pixel/) (원래 #31과 #32~#36)

최소한의 Swift 셸을 세웁니다.

- Swift Package 래핑, ABI 핸드셰이크 배선
- AppDelegate → WindowController → SessionHost + TerminalView 뼈대
- 그리드는 80×24 고정이고 창은 리사이즈하지 않습니다. 리플로우를 M3의 리사이즈까지 미룹니다
- wake 결합, 디스플레이 링크 dirty 게이팅
- 렌더러 최소: 2패스 파이프라인, ASCII fast path만, 아틀라스 1페이지
- 렌더러 골든 1종 — 하네스 녹화를 detached 세션에 먹여 인스턴스 버퍼를 비교합니다. CI에 `swift build`·`swift test`가 붙습니다
- **종료 기준**: 셸 프롬프트가 보이고 `ls` 출력이 맞습니다. 유휴 시 링크 정지 확인

## M3 — Input and text

스펙과 티켓: [`.scratch/m3-input-and-text/`](../.scratch/m3-input-and-text/)

선행 조건이던 GSUB 프로브는 끝났고 [R3](04-renderer.md#r3--shaping-unit)의 셰이핑 경로는 [0016](adr/0016-derive-the-ligature-path.md)에서 확정되었습니다.

- 입력 경로 전체 배선 (쓰기 / 붙여넣기 / 키 / 휠 / 마우스 / 포커스 / **리사이즈**). 경계를 건너는 것은 의미 이벤트이고 판정은 코어입니다. cf. [0017](adr/0017-semantic-input-events.md)
- NSTextInputClient + preedit 오버레이, Option-as-Meta
- 셰이핑 slow path(결합·ZWJ·이모지·**리거처**), 폴백, 정수 정렬, 선택 + 복사
- **Bold·Italic 4종 face.** M2는 한 벌만 그립니다. 프롬프트와 하이라이트 대부분이 bold라 이것 없이는 "일상 사용 가능"이 성립하지 않습니다
- **종료 기준**: IME 시나리오 전항 통과, vim/tmux/fzf 일상 사용 가능
    - 자가 데일리 드라이버 전환 시점입니다. 이후로는 실사용으로 버그를 수집합니다.

## M4 — Daily driver items

- **설정 파이프라인이 뿌리입니다** — 폰트·테마·Option-as-Meta·벨. 나머지 정책은 상수입니다
- 이벤트 소비 정책(벨·클립보드·종료), **창 여럿(⌘N·⌘W)**, 창 복원
- ⌘ URL 스캔, 종료 경고, ⌘K, 밑줄 데코레이션, 아틀라스·재드로우 리셋 경로
- tmux 수준 1 표 전 항목 마감
- **종료 기준**: [06-integration](06-integration.md) 표 전항 + [DoD A절](07-definition-of-done.md#a-features) 완료

## M5 — Closing

- 성능 게이트 전항. 미달 항목은 렌더 스레드 승격을 검토합니다
- DoD E·F 전항, 퍼저 24시간, 하네스 10종 확충
- [open-questions](open-questions.md)의 v1 절이 비어 있음을 확인
- **종료 기준**: [07-definition-of-done](07-definition-of-done.md) 전 항목 체크 = v1 태그

## Between milestones

- 헤더는 M0 이후 동결 상태입니다. 생성 결과가 헤더와 다르면 그것은 결함이며, 고칠 곳은 헤더가 아니라 Rust 쪽 어노테이션입니다.
- 각 마일스톤에서 발견된 사양 결함은 해당 챕터를 수정하고 ADR을 추가합니다. 챕터와 ADR의 역할 분리를 유지합니다.
- **VT 엔진 버전 올리기는 마일스톤 사이에서만** 합니다. 하네스 통과가 승인 조건입니다.
