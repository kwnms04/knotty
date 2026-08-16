# Architecture

## Layers

| 층 | 소유 | 비고 |
|---|---|---|
| **VT 엔진** (`libghostty-vt`) | 파싱, 터미널 상태, 스크롤백, 리플로우, 셀 폭, 선택, 입력 인코딩 | 외부 라이브러리. 타입은 경계를 넘지 않음 |
| **knotty-core** (Rust) | PTY, I/O 스레드, writer 큐, 이벤트 큐, 스냅샷 변환, 메일박스 | OS 비의존 로직이 여기 모임 |
| **knotty-config** (Rust) | TOML 파싱·검증·기본값 병합 | 경계로는 직렬화 블롭 하나만 |
| **knotty-ffi** (C ABI) | 유일한 언어 경계 | 생성된 헤더가 진실 |
| **App/** (Swift) | AppKit + Metal, IME, 입력, 레이아웃, 테마, 정책 | macOS 전용. hot path 접근 불가 |

VT 엔진과 knotty-core는 둘 다 "코어"로 불리기 쉽습니다. 이 문서에서 **VT 엔진**은 `libghostty-vt`를, **코어**는 knotty-core를 가리킵니다.

관련: [0001](adr/0001-portable-core.md) 이식 가능한 코어 · [0002](adr/0002-libghostty-vt-core.md) VT 엔진 선택 · [0004](adr/0004-hide-vt-engine-types.md) 타입 은닉 · [0012](adr/0012-own-the-binding-layer.md) 바인딩 계층 소유

## 3 Flows

상세는 [02-ffi](02-ffi.md)로.

1. **입력 경로** (App → 코어, 명령): 모든 입력 함수는 블록되지 않으며(detached 전용 `feed`는 예외 — 호출자 스레드 동기 처리), 쪼개지지 않는 하나의 단위로 처리됩니다.
2. **출력 경로** (코어 → App, 스냅샷): 코어가 만들어 단일 슬롯 메일박스에 게시하고, App은 wake가 있던 프레임에 받아갑니다.
3. **이벤트 경로** (코어 → App, 통지): 인자 없는 wake 신호만 보내고, 실제 내용은 따로 꺼내며, 놓치면 안 되는 사실만 담습니다.

## Inside the core (not crossing boundary)

1. PTY 읽기 → VT 엔진 → 스냅샷 변환 → 메일박스: 렌더 속도와 무관하게 진행됩니다.
2. VT 엔진 → PTY 응답 (DA1/DSR, 색 질의 응답): 사용자 입력과 같은 writer 큐를 거칩니다. 이 경로에 [응답 위생 필터](adr/0007-input-security.md)가 걸립니다.

## Ownership

| 코어 (Rust) | 앱 (Swift) |
|---|---|
| 세션 집합 (평평한 핸들 목록) | 레이아웃 · 포커스 |
| 스냅샷 변환과 게시 | 글리프 · 셰이핑 · 폴백 (CoreText) |
| 팔레트 런타임 상태 | 테마 — 초기값 주입 주체이자 시각 정체성 |
| TOML 파싱 · 검증 · 기본값 | 설정 적용 · 파일 감시 |
| 응답 위생 (질의 반사 차단) | 제스처 해석, 클립보드 허용 정책, URL 클릭 정책 |

레이아웃은 UI 문제입니다.
셀 폭은 렌더링 문제가 아니며, VT 엔진이 판정합니다.

## Session model

- 세션당 단일 창을 가집니다. 탭도 멀티플렉싱의 일부로 간주합니다. cf. [0009](adr/0009-no-multiplexer.md) · [0010](adr/0010-no-tabs-v1.md)
- 세션은 두 가지로 생성됩니다.
    - PTY가 있는 세션 — I/O 스레드를 소유합니다.
    - PTY가 없는 **detached** 세션 — 스레드가 없고 `feed`가 호출자 스레드에서 동기 실행됩니다.
- 파서 이후의 경로는 둘이 동일합니다. 메일박스도 함께 지납니다. cf. [0008](adr/0008-detached-session-public.md)
