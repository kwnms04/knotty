# Knotty

macOS Terminal Emulator w. Rust, Swift

## Goal

**Daily driver**.
macOS 통합 품질에서 차별화합니다.

- e.g. IME, 네이티브 창 동작, 배터리, 지연, etc.

v1의 검증 대상은 macOS뿐이지만, **코어는 다른 OS로 이식 가능한 상태를 유지합니다**. OS 비의존 로직은 Rust 코어에, macOS 전용은 Swift에 둡니다. cf. [ADR 0001](adr/0001-portable-core.md)

## Discipline

1. **차별화되지 않는 부분은 만들지 않습니다.** 검증된 라이브러리를 먼저 사용합니다.
2. **단일 언어 경계(C ABI)를 가집니다.** hot path에 Swift는 없습니다.
3. **헤드리스 검증이 가능해야 합니다.**
4. **드문 이벤트를 위해 상시 비용을 내지 않습니다.**
5. **되돌리기 비싼 결정을 우선시합니다.**

## Document rules

- **모든 챕터는 항상 최신화됩니다.** 단, ADR은 그러지 아니합니다.
- **Github Flavored Markdown을 준수합니다.**
- 결정의 근거는 ADR에, 현재 상태는 챕터에 둡니다. 챕터가 ADR과 어긋나면 챕터가 틀린 것입니다.
