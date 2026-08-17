---
status: accepted
---

# 바인딩의 safe 계층은 knotty가 소유한다

`libghostty-vt-sys`(bindgen 생성)에만 의존하고, 그 위의 safe 경계는 knotty가
직접 씁니다. 서드파티 safe 래퍼 크레이트 `libghostty-vt`는 걷어냅니다. VT 엔진
선택은 바뀌지 않습니다 — [0002](0002-libghostty-vt-core.md)는 그대로 유효합니다.

## 근거

[0002](0002-libghostty-vt-core.md)는 safe 래퍼가 서드파티임을 인정하면서
"필요하면 `ffi` 모듈로 sys에 직접 내려갈 수 있다"는 탈출구를 남겼습니다. 이
ADR은 그 탈출구를 예외에서 기본 경로로 승격시킵니다.

계기는 `ClipboardWrite::contents`의 널 슬라이스
([uzaaft/libghostty-rs#74](https://github.com/Uzaaft/libghostty-rs/issues/74))
였고, 그 뒤 knotty 경로에 걸리는 원시 포인터 지점을 전수 감사했습니다. 결과는
버그 한 건이 아니라 **계층의 분리**였습니다.

| 계층 | 규모 | 확인된 불건전 |
|---|---|---|
| `libghostty-vt` (손으로 쓴 safe 래퍼) | 9,200줄, `unsafe` 369회 | **3건** |
| `libghostty-vt-sys` (bindgen 생성) | 320K | 0건 |

세 건 모두 한 패턴입니다: **원시 C 데이터를 safe 타입으로 즉시 변환하고, 필드는
private이며, 원시 접근자가 없다.** 그래서 계약 불일치가 전부 다운스트림이 방어할
수 없는 UB가 됩니다.

- `terminal.rs:1271` `ClipboardWrite::contents` — C가 "비우기"를 뜻해 보내는
  널 배열로 슬라이스를 만듭니다. 디버그 빌드 중단
- `libghostty-vt-sys` `lib.rs:41` `String::to_str` — `from_utf8_unchecked`.
  C 헤더가 "binary-safe"라고 정의한 클립보드 데이터를 검증 없이 `&str`로
  만듭니다. `ESC ] 52 ; c ; / / 4 = BEL` 12바이트로 재현되며, **릴리스에서
  조용한 UB**
- `render.rs:822` `graphemes_buf` — `&mut [char]`를 받고 C에는 포인터만 넘겨
  길이를 전달하지 않습니다. 버퍼 1칸에 3칸을 씁니다. safe 함수에서 힙 오버플로
  ([#70](https://github.com/Uzaaft/libghostty-rs/issues/70))

반면 `-sys`는 C 헤더와 컴파일 타임에 대조됩니다. 레이아웃이 어긋나면 빌드가
깨지므로, 손으로 쓴 계층이 낸 종류의 실수를 낼 수가 없습니다.

```rust
["Size of ClipboardContent"][size_of::<ClipboardContent>() - 32usize];
["Offset of field: ClipboardContent::mime"][offset_of!(ClipboardContent, mime) - 0usize];
```

knotty가 v1에 닿는 표면은 래퍼 9,200줄의 절반이 안 됩니다. 파사드는 그중
knotty가 실제로 부르는 것만 덮으므로 **800~1,500줄** 규모입니다.

혐의를 벗은 지점도 기록해 둡니다. `Terminal::title`·`pwd`는 빈 문자열 9경로
(`OSC 0/1/2/7`, 설정 후 비우기 포함)에서 전부 동일한 비-널 정적 sentinel을
반환하고 `std::str::from_utf8` 검증까지 하므로 건전합니다. `GhosttyString.ptr`은
C 헤더에서 non-optional이며, 이것이 `ClipboardWrite.contents`의
`?[*]const ClipboardContent`와 다른 점입니다.

## 기각안

**래퍼 유지 + 패치.** 상류는 실제로 응답합니다 — 종료된 이슈 다수가 당일~수일,
건전성 이슈 [#70](https://github.com/Uzaaft/libghostty-rs/issues/70)에는 4일 만에
기술적 반박까지 달렸습니다. 유지보수 실패는 근거가 아닙니다. 기각 사유는 다른
데 있습니다: 세 건이 같은 구조적 패턴에서 나왔고, 그 패턴이 남아 있는 한 다음
건도 knotty가 방어할 수 없습니다. `-sys`는 계속 씁니다.

**래퍼 재작성.** `unsafe`는 사라지지 않고 소유자만 바뀌는데, 리뷰어가 366-star
사용자 기반에서 1명으로, 검증 이력이 0으로 줄고, 상류 수정 수혜가 사라집니다.
건전성을 개선하지 않고 역행시킵니다. 파사드와의 차이는 **표면 크기**입니다 —
재작성은 9,200줄을 상대하고 파사드는 knotty가 부르는 것만 덮습니다.

**Zig 직행.** ghostty는 `src/lib_vt.zig`를 Zig 모듈로 내보내고, Zig 0.15.x는
knotty가 이미 빌드에 요구합니다. 그럼에도 기각합니다.

- `lib_vt.zig`(2025-09-20)와 `include/ghostty/vt.h`(2025-09-24)는 **나흘 차이의
  동갑**입니다. 검증된 것은 그 아래 `src/terminal/`이고, 두 공개 표면은 같은
  추출 작업의 산물입니다. "검증된 쪽으로 간다"가 성립하지 않습니다
- `lib_vt.zig`는 첫 주석에서 *"the API itself may change without warning"*이라고
  **스스로 안정성을 거부합니다.** C 쪽은 버전 헤더 + ABI 검사 + `GHOSTTY_COMMIT`
  핀을 답니다. [0002](0002-libghostty-vt-core.md)가 지정한 완화책(핀 + 골든
  하네스)은 API가 열거 가능할 때만 작동합니다
- 착지점이 없습니다. **Zig shim → C ABI → Rust 코어 → `knotty.h`**는 변환을 두
  번 지납니다. **Zig 정적 링크 + Rust가 원시 구조체 읽기**는 세 버그를 만든 그
  연산을 `knotty-core`(현재 `unsafe` 0)로 들여옵니다. **Rust 코어 소멸**은
  2,476줄 재작성이며 borrow checker를 전부 포기합니다 — 건전성을 이유로 한
  선택으로서 논리가 뒤집힙니다

**VT 엔진 자작.** `src/terminal/`은 122,722줄에 test 블록 2,376개입니다. 파서는
싼 부분이고(DEC ANSI 상태 기계 ~500줄), 비싼 것은 `PageList`의 페이지 기반
스크롤백과 압축, 리사이즈 리플로우, grapheme 클러스터링과 wcwidth입니다.
[0002](0002-libghostty-vt-core.md)의 원칙 1("차별화되지 않는 부분은 만들지
않는다")이 문자 그대로 금지합니다.

## 결과

- **`knotty-core`에 `unsafe`가 들어옵니다.** 오늘은 0입니다. 파사드가 그
  경계이며, 여기서 `unsafe`가 늘어나는 것은 설계대로입니다. 대신 그것은 knotty가
  감사한 코드이고, `knotty-core`의 나머지와 `knotty-ffi` 바깥은 계속 0입니다
- **`-sys`는 여전히 서드파티입니다.** `GHOSTTY_COMMIT` 핀, bindgen 재생성,
  플랫폼별 빌드 경로는 같은 유지보수자에게 계속 의존합니다. 절반만 떠나는
  것입니다
- **상류 수정을 자동으로 받지 못합니다.** 래퍼가 고쳐져도 파사드는 knotty가
  갱신합니다. 대신 [0002](0002-libghostty-vt-core.md)의 골든 하네스가 그대로
  안전망입니다 — 엔진 버전 올리기가 무엇을 깨뜨렸는지는 여전히 드러납니다
- **파사드는 M1 종료 후, M2 착수 전에 넣습니다.** M1의 종료 기준이 "퍼저 1시간
  무크래시"인데 현재 래퍼로는 달성 불가능합니다(#74에서 멈추고, UTF-8 쪽은
  크래시가 아니라 퍼저가 잡지도 못합니다). 그때까지는 `[patch.crates-io]`
  스톱갭으로 진행합니다: `contents` 널 검사, `ClipboardContent`의 `mime`·`data`를
  `&[u8]`로. 이 타입 변경은 되돌리지 않습니다 — 파사드도 바이트로 내보냅니다
- **스톱갭은 절반만 회수됩니다.** 건전성 수정 둘은 `libghostty-vt` 안에 있으므로
  파사드가 그 크레이트를 걷어낼 때 함께 사라집니다. 확인 가능한 조건은 두
  가지입니다 — `[patch.crates-io]`에 `libghostty-vt` 항목이 없고, 파사드가
  `String::to_str`을 부르지 않습니다(파사드는 `ffi::String`을 직접 읽어 자기
  검증을 합니다).

  `-sys` 항목은 **영구히 남습니다.** 빌드 훅(`ZIG`, `ZIG_SYSROOT`)이 거기 있고,
  파사드 이후에도 `-sys`를 계속 쓰기 때문입니다. 상류 PR과 shim 회귀를 두고
  고른 결과입니다.

  근거는 **패치에 남은 것이 무엇인가**입니다. 파사드가 `libghostty-vt`를
  걷어내면서 건전성 수정 둘이 함께 나갔으므로, 브랜치의 diff는 이제
  `build.rs`뿐입니다. 이것은 영구히 들고 갈 만한 종류입니다 — 상류가 흔들리면
  빌드가 그 자리에서 깨지지, 건전성 수정처럼 조용히 어긋나지 않습니다. shim
  회귀는 `xcrun`을 `PATH`에 깔아 셸 전체에 영향을 주므로 AGENTS.md가 이미 더
  나쁜 쪽으로 기록해 두었습니다.

  **비용은 엔진 bump마다 반복됩니다.** `-sys`는 핀 박힌 ghostty 커밋에 대고
  생성되므로, 엔진을 올릴 때마다 상류의 새 릴리스 위로 브랜치를 다시 따야
  합니다. 상류 PR은 그 비용을 한 번만 낼 수 있었을 것이고, 이 결정은 매번
  냅니다. 훅 둘뿐이라 회당 비용이 작다는 것이 이 교환의 전부입니다.

  **그러므로 이 결정에는 확인 가능한 완료 조건이 없습니다.** 다른 항목과 달리
  `[patch.crates-io]`의 `-sys` 줄이 남아 있는 것이 정상 상태이며, 지워야 할
  잔재가 아닙니다. 뒤집으려면 상류에 훅을 올리고, 병합·crates.io 릴리스·`=0.2.1`
  핀 이동(마일스톤 사이에서만 하는 일)까지 끝난 뒤 이 문단부터 고칩니다.
- **비-UTF-8 클립보드 쓰기는 코어에서 거절합니다.** `session.rs`가
  `text/plain` 표현만 받으므로 UTF-8이 아닌 것은 그 MIME 타입으로서 malformed
  입니다. `ClipboardWriteError::InvalidData`로 돌려보내며, 이는
  `CLIPBOARD_TEXT_CAP` 초과를 `Denied`로 막는 기존 동작과 같은 층위입니다.
  `KtEvent.text`가 `KtText`("Borrowed UTF-8")인 채로 유지되고 헤더는 변하지
  않습니다. v2에서 rich clipboard가 들어와 `text/plain` 아닌 표현을 받게 되면
  `KtBytes`로의 전환을 다시 검토합니다
- **파사드는 "이름만 바꾸는 계층"처럼 보입니다.** [0004](0004-hide-vt-engine-types.md)
  가 변환 계층에 대해 경고한 것과 같은 유혹이 생깁니다. 지우고
  `libghostty-vt`로 갈아끼우려면 이 ADR부터 뒤집어야 합니다
