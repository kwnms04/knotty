# Milestones

아키텍처가 강제하는 순서를 따릅니다. 하네스가 코어의 첫 소비자이고, 그다음은 "첫 픽셀까지 최단"입니다.
각 마일스톤은 기간이 아니라 **종료 기준**으로 정의합니다. 종료 기준에 없는 것은 하지 않습니다.

## M0 — Headless core

Swift 없이 knotty-core와 하네스만 만듭니다.

스펙과 티켓: [#1](https://github.com/kwnms04/knotty/issues/1) 및 그 하위 이슈 (#2~#9)

- detached 세션 + `feed` + 메일박스 + 평탄 POD 스냅샷 변환
- 헤더 생성, ABI 핸드셰이크, 불투명 핸들, 패닉 격리
- 하네스: 녹화 스트림 3종(vim, tmux, htop) 골든 통과
- **종료 기준**: 하네스 CI 녹색, 스냅샷 형식이 생성된 헤더와 바이트 단위로 일치

## M1 — PTY and threading

- I/O 스레드 ([C1](03-core.md#c1--thread-topology)), PTY 스폰, writer 큐 + 배압
- 자식 종료 처리 + 순서 불변식 ([C6](03-core.md#c6--child-lifecycle))
- 이벤트 큐 3종 (벨, 클립보드 쓰기, 자식 종료)
- **응답 위생 필터** ([C4](03-core.md#c4--listener-and-response-hygiene)) — PTY가 생겨야 검증 가능합니다
- `feed` 퍼저
- **종료 기준**: B4·B5 벤치 통과, 퍼저 1시간 무크래시

## M2 — First pixel

최소한의 Swift 셸을 세웁니다.

- Swift Package 래핑, ABI 핸드셰이크 배선
- AppDelegate → WindowController → SessionHost + TerminalView 뼈대
- wake 결합, 디스플레이 링크 dirty 게이팅
- 렌더러 최소: 2패스 파이프라인, ASCII fast path만, 아틀라스 1페이지
- **종료 기준**: 셸 프롬프트가 보이고 `ls` 출력이 맞습니다. 유휴 시 링크 정지 확인

## M3 — Input and text

**선행 조건: GSUB 프로브.** [R3](04-renderer.md#r3--shaping-unit)의 셰이핑 경로가 확정되어야 합니다.

- 입력 경로 전체 배선 (쓰기 / 붙여넣기 / 특수키 / 휠 / 마우스 / 포커스)
- NSTextInputClient + preedit 오버레이, Option-as-Meta
- 셰이핑 slow path(결합·ZWJ·이모지·**리거처**), 폴백, 정수 정렬, 선택 + 복사
- **종료 기준**: IME 시나리오 전항 통과, vim/tmux/fzf 일상 사용 가능
    - 자가 데일리 드라이버 전환 시점입니다. 이후로는 실사용으로 버그를 수집합니다.

## M4 — Daily driver items

- 이벤트 소비 정책(벨·클립보드·종료), 창 복원, 설정 파이프라인
- ⌘ URL 스캔, 종료 경고, 아틀라스·재드로우 리셋 경로
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
