# Swift App

## 1 — Single process

코어는 정적 링크합니다. 헬퍼 프로세스로 분리하지 않습니다.
크래시 격리는 패닉 포획([C8](03-core.md#c8--panic-isolation))과 퍼징이 대체합니다.
데일리 드라이버에는 격리보다 기동 속도(⌘N 즉시)가 중요합니다.

## 2 — Build and targets

cf. [0014](adr/0014-swiftpm-no-xcodeproj.md)

- `.xcodeproj`는 두지 않습니다. `App/Package.swift` 하나가 진실이고, `.app` 번들과 `default.metallib`, ad-hoc 서명은 조립 스크립트가 만듭니다.
- 타깃은 넷이며, 경계가 곧 규율입니다.

```
CKnotty        systemLibrary. include/knotty.h를 보는 유일한 타깃
KnottySession  FFI 파사드. CKnotty를 import하는 유일한 타깃
KnottyRender   스냅샷+메트릭 → 인스턴스 버퍼·아틀라스 갱신 목록.
               AppKit도 MTLDevice도 참조하지 않음 (R9)
knotty         AppKit, Metal 인코딩·드로우, 소유 트리
```

- `TerminalView`가 FFI를 직접 부를 수 없다는 [3절](#3--ownership-tree)의 계약과, 렌더러가 순수 함수라는 [R9](04-renderer.md#r9--pure-function)의 계약이 여기서 빌드 실패가 됩니다. 집행하는 것은 타깃 그래프가 아니라 조립 스크립트입니다. cf. [0015](adr/0015-boundary-check-in-the-script.md)
- 렌더러 골든은 GPU 없이 `swift test`로 돕니다. detached 세션([0008](adr/0008-detached-session-public.md))이 공개 ABI이므로 Rust 하네스와 같은 녹화 파일을 그대로 먹입니다.

## 3 — One session, one window

- 탭은 제공하지 않습니다. cf. [0010](adr/0010-no-tabs-v1.md)
- 시스템 설정("탭으로 열기 선호")이 창을 자동으로 탭 그룹에 묶지 않도록 탭 병합을 명시적으로 비활성화합니다.
- 나중에 도입한다면 그 설정을 바꾸는 것만으로 가능합니다. 자체 탭 구현이 아닙니다.

## 4 — Ownership tree

```
AppDelegate (설정 스토어, 세션 레지스트리)
└─ TerminalWindowController (창마다)
   ├─ SessionHost   ← 세션 핸들을 만지는 유일한 객체
   │                  (이벤트 드레인, 스냅샷 수령, 렌더러 소유)
   │                  스냅샷은 스코프 안에서만 유효하므로, 그것을
   │                  프레임으로 바꾸는 렌더러도 여기에 있습니다
   └─ TerminalView  ← NSView: Metal 레이어, NSTextInputClient, 제스처 상태
                      FFI 직접 호출 금지 — SessionHost에 의도 전달만
```

"세션당 호출 직렬화" 계약을 규율이 아니라 **구조**로 보장합니다.
FFI를 아는 객체가 하나면 동시 호출 경로 자체가 없습니다.

**뷰가 가진 "제스처 상태"는 앵커 셀·클릭 수·드래그 중인지 셋뿐입니다.**
무엇이 선택되는지는 코어가 정합니다 — 워드·라인 경계는 엔진의 유니코드
규칙이고, 뷰는 그것을 다시 계산하지 않습니다. cf.
[0017](adr/0017-semantic-input-events.md)

## 5 — Main thread only

cf. [0011](adr/0011-main-thread.md)

- wake 콜백(코어 I/O 스레드)은 자기 스레드를 깨우는 것만 합니다. 헤더가 허용하는 유일한 행위입니다.
    - 메인 큐 핸들러가 드레인 → 스냅샷 수령 → 렌더 준비를 수행합니다.
- 프레임당 비용에서 드레인·스냅샷 수령·Metal 인코딩은 무시할 수 있습니다.
    - 진짜 비용은 셰이핑뿐이고 [R2](04-renderer.md#r2--full-redraw-damage-saves-shaping)·[R3](04-renderer.md#r3--shaping-unit) 캐시로 통제합니다.
- 성립 조건 셋:
    - FFI 쓰기 함수는 블록하지 않습니다. 흐름 제어로 자식이 읽지 않아도 메인 스레드는 무사합니다.
    - **스냅샷 수령이 O(1)입니다.** 완성된 값을 메일박스에서 받아가므로 게시 측과 경쟁하지 않습니다.
    - 렌더러는 순수 함수입니다. 수치가 깨지면 렌더 스레드 승격이 디스패치 한 줄입니다.
- 디스플레이 링크는 런루프에 통합되는 쪽이 1순위, 자체 스레드 콜백 방식이 폴백입니다.
- 메인을 떠날 수 없는 것: NSTextInputClient 전부, 이벤트 처리, 창 조작, SessionHost의 FFI 호출.

## 6 — Render loop

wake → `needsFrame` + 링크 재개 → vsync 틱에 스냅샷 수령 → dirty 줄만 셰이핑 → 아틀라스 갱신 →
인코딩 → 그리기 → 새 wake가 없으면 링크 일시정지.

"wake 있던 프레임만 수령"과 "유휴 CPU 0"이 구조적으로 성립합니다.
폭주는 vsync가 자연 스로틀합니다.

## 7 — IME

- marked text는 코어로 보내지 않습니다. 커서 위 Swift 오버레이로 그리고, 확정 시에만 SessionHost를 거쳐 쓰기를 호출합니다. TerminalView는 여기서도 FFI를 직접 부르지 않습니다.
    - 미확정 텍스트는 터미널에 입력된 것이 아닙니다. 그리드에 넣으면 취소 되돌리기가 지옥이 되고 PTY 부분 전송은 재앙입니다.
- 후보창 위치는 스냅샷 커서 좌표에서 계산합니다.
- keyDown은 IME에 먼저 기회를 줍니다. 미소비 키만 SessionHost를 거쳐 특수키 경로로 갑니다.
    - Option-as-Meta는 **엔진의 인코더 옵션**입니다. 앱은 설정(좌/우 개별)을
      코어에 넘길 뿐, 어떤 바이트가 나갈지 판정하지 않습니다. cf.
      [0017](adr/0017-semantic-input-events.md)

## 8 — Event policy

| 이벤트 | 정책 |
|---|---|
| 벨 | 설정: 사운드 / 독 바운스 / 시각 벨. 비활성 창은 배지 |
| 클립보드 쓰기 | 삼단: 항상 허용 / 확인(기본, 세션별 기억) / 차단 |
| 자식 종료 | 코드 0이면 창 닫기, 비정상이면 유지 + 코드 표시. 설정 가능 |
| 붙여넣기 경고 | 검사 결과가 멀티라인 또는 제어 문자 포함이면 경고 시트(미리보기 포함). 설정: 항상 / 멀티라인만(기본) / 끔 |

종료 경고("실행 중 프로세스")는 스냅샷의 자식 상태로 판정합니다.
**경고는 끌 수 있지만 정화는 끌 수 없습니다.** cf. [0007](adr/0007-input-security.md)

## 9 — State restoration

- 창 프레임을 복원하고, 세션은 저장된 작업 디렉터리에서 새 셸을 스폰합니다.
    - 스냅샷의 작업 디렉터리를 주기적으로 저장합니다. 이를 전달하지 않는 tmux 세션은 홈에서 시작합니다. 문서화된 한계입니다.
- 스크롤백은 복원하지 않습니다. tmux 사용자는 `tmux attach`가 복원 수단입니다.

## 10 — Config pipeline

- ConfigStore: 로드 → JSON → Codable → 창들에 발행.
- 파일 감시(디바운스 ~200ms) → 리로드 → diff → 항목별 적용.
    - 폰트·테마는 즉시 적용합니다. 테마 변경은 팔레트 재주입이며 [R8](04-renderer.md#r8--reset-triggers)의 "재드로우만" 경로입니다.
    - 스크롤백 크기는 **새 세션부터** 적용합니다. 기존 세션 변경 API는 두지 않습니다.
