# VT 바인딩 파사드: libghostty-vt를 걷어내고 -sys를 직접 감싼다

Blocked by: 08, 13
Was: #27

## What to build

`knotty-core/src/vt/`가 `libghostty-vt-sys`를 직접 감싸고, 서드파티 safe 래퍼
크레이트 `libghostty-vt`는 의존성에서 빠집니다. 근거와 기각안은
[0012](https://github.com/kwnms04/knotty/blob/main/docs/adr/0012-own-the-binding-layer.md).

파사드는 접근자를 1:1로 다시 내보내는 계층이 아닙니다. 평탄화 루프가 그 안으로
들어갑니다 — `snapshot.rs`의 행 × 셀 순회가 `vt` 모듈로 옮겨가고, 코어에는 엔진
타입이 하나도 남지 않습니다. 접근자를 20개 재수출하는 형태로 만들면
[0004](https://github.com/kwnms04/knotty/blob/main/docs/adr/0004-hide-vt-engine-types.md)가
경고한 "이름만 바꾸는 계층"이 실제로 생기고, `graphemes_len` 없이
`graphemes_buf`를 부르는 종류의 계약을 파사드가 강제할 수 없게 됩니다.

덮을 표면은 오늘 코어가 부르는 것뿐입니다 — 함수 28개, 타입 약 15개. 입력
인코딩은 M3에서 그 경로가 생길 때 붙입니다.

`-sys` 패치는 남습니다. 빌드 훅(`ZIG`, `ZIG_SYSROOT`)이 거기 있고 파사드
이후에도 `-sys`를 씁니다. 상류 PR·영구 유지·shim 회귀 중 무엇으로 갈지는 이
티켓에서 결정합니다.

## Acceptance criteria

- [x] `knotty-core/src/vt/`가 `libghostty-vt-sys`를 감싸고, 엔진 타입이 그 모듈 밖으로 나가지 않는다
- [x] 평탄화(`capture`)가 그 모듈 안에 있다
- [x] `knotty-core`에 `#![deny(unsafe_code)]`가 걸리고, `#[allow(unsafe_code)]`는 `vt` 모듈 하나뿐이다
- [x] `libghostty-vt`가 의존성과 `[patch.crates-io]` 양쪽에서 사라진다
- [x] 파사드가 `String::to_str`을 부르지 않는다
- [x] `-sys` 패치의 거취가 결정되고 [0012](https://github.com/kwnms04/knotty/blob/main/docs/adr/0012-own-the-binding-layer.md)에 반영된다
- [x] 골든 넷이 갱신 없이 통과한다
- [x] 생성된 헤더가 변하지 않는다
- [x] `03-core.md`의 모듈 목록에 `vt`가 들어간다
