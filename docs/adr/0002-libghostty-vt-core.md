---
status: accepted
---

# VT 엔진은 libghostty-vt를 쓴다

파싱·터미널 상태·스크롤백·리플로우·선택·입력 인코딩은 `libghostty-vt`에 위임하고, Rust 코어는 그 위의 오케스트레이션만 담당합니다. 이전 계획은 `alacritty_terminal`을 래핑하는 것이었으며, 이를 대체합니다.

## 근거

이 프로젝트의 원칙 1은 "차별화되지 않는 부분은 만들지 않는다"입니다. `libghostty-vt`는 `alacritty_terminal`보다 비차별화 영역을 더 많이 흡수합니다. 아래 셋은 이전 계획에서 knotty가 직접 만들 예정이던 것입니다:

| 항목 | libghostty-vt |
|---|---|
| 모드 의존 키·마우스·포커스 인코딩 (DECCKM 등) | 제공 |
| 스냅샷 + 변경된 줄 증분 | 증분 렌더 상태로 제공 |
| 선택 상태를 셀 단위 배열로 전개 | 선택 제공 |

이식성([0001](0001-portable-core.md))에서도 우위입니다. `libghostty-vt`는 무의존성이며 libc조차 요구하지 않고, macOS·Linux·Windows·WebAssembly를 커버합니다. `alacritty_terminal`은 std를 요구합니다.

## 기각안

- **`alacritty_terminal` 유지** — 검증됐고(0.17.0) API도 더 안정적이지만, 위 표의 항목을 knotty가 전부 직접 만들어야 합니다.
- **Rust 삭제 후 Swift가 C API 직접 호출** — [0001](0001-portable-core.md)이 기각합니다.

## 결과

- Rust 바인딩(`libghostty-vt` 크레이트)은 **서드파티**입니다. ghostty-org 공식이 아니라 커뮤니티가 유지하며, `libghostty-vt-sys`를 감싼 안전 래퍼입니다. 필요하면 `ffi` 모듈로 sys에 직접 내려갈 수 있습니다.
- **API가 불안정합니다** (퍼블릭 알파). 완화책은 버전 고정 + 헤드리스 하네스의 골든 스냅샷입니다. 녹화된 터미널 출력 스트림을 재생해 결과를 고정된 기대값과 비교하므로, 업그레이드가 무엇을 깨뜨렸는지 즉시 드러납니다.
- 안전 래퍼의 핸들이 `!Send + !Sync`라는 점이 스레드 토폴로지를 강제합니다. [0003](0003-snapshot-mailbox.md) 참조.
