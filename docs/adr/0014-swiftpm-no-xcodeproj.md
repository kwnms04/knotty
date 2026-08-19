---
status: accepted
---

# Swift 쪽은 SwiftPM 하나로 짓고, 경계는 타깃으로 나눈다

`App/Package.swift`가 Swift 쪽의 유일한 빌드 정의입니다. `.xcodeproj`는 두지
않고, `.app` 번들과 `default.metallib`과 ad-hoc 서명은 조립 스크립트가 만듭니다.
타깃은 넷이며, `CKnotty` → `KnottySession` → `KnottyRender` → `knotty` 순으로만
의존합니다.

## 근거

이 저장소가 Swift 쪽에 요구하는 것 중 Xcode가 거저 주지 않는 것이 하나 있습니다.
[R9](../04-renderer.md#r9--pure-function)는 렌더러를 순수 함수로 두는 효용
둘 중 하나로 "GPU 없는 CI에서 렌더러 골든 테스트가 가능한 것"을 들었고,
[DoD D절](../07-definition-of-done.md#d-verification-infrastructure)은 그것을
산출물 목록에 올려 두었습니다. 그 테스트가 `swift test` 한 줄로 도는 형태는
렌더러가 라이브러리 타깃일 때이고, `.xcodeproj`는 같은 테스트를 `xcodebuild`
뒤로 옮깁니다.

타깃 경계를 넷으로 쪼갠 이유는 [05-swift-app 4절](../05-swift-app.md#4--ownership-tree)이
쓴 것과 같습니다 — "규율이 아니라 **구조**". Swift에서 그 구조를 만드는 수단이
타깃 의존 그래프입니다. 렌더러 안에서 `kt_session_write`를 부르는 코드도,
`TerminalView`가 FFI를 직접 부르는 코드도, 그 타깃이 의존에 적지 않은 모듈을
들여와야 존재할 수 있습니다.

**단, 그 import를 막는 것은 그래프가 아닙니다.** M2에서 확인했습니다 — AppKit은
SDK 모듈이라 어디서든 들어오고, systemLibrary의 모듈 맵은 전이하므로
`import CKnotty`도 어디서든 컴파일됩니다. 경계 넷은 그대로 두고 집행만 조립
스크립트로 옮겼습니다. cf. [0015](0015-boundary-check-in-the-script.md)

Xcode가 거저 주던 것 — 번들 레이아웃, 셰이더 컴파일, 서명 — 은 스크립트 몇
줄로 되삽니다. 셰이더는 확인하고 결정했습니다: SwiftPM은 `.metal`을
컴파일하지 않고 리소스로 복사만 하므로, 어느 쪽을 골라도 누군가는 `xcrun metal`을
불러야 합니다.

## 기각안

**`.xcodeproj` 커밋.** 번들·셰이더·서명이 전부 딸려 오고, 기각 사유는 두
가지입니다. `pbxproj`가 저장소의 진실이 되면 파일 추가가 손으로 병합하기 나쁜
형식의 변경이 되고, 위에 쓴 대로 렌더러 골든이 `xcodebuild` 뒤로 들어갑니다.
Xcode가 실제로 필요해지는 시점 — 노터라이즈, Instruments 프로파일 — 은 M5이며,
그때 SwiftPM 패키지를 감싸는 얇은 프로젝트를 얹는 편이 지금부터 `pbxproj`를
들고 가는 것보다 쌉니다. **이 결정을 뒤집는 자리가 거기입니다.**

**XcodeGen.** `project.yml`이 진실이고 `.xcodeproj`는 생성물이므로 병합 문제는
풀립니다. 대신 도구 의존이 하나 늘고, 테스트가 `xcodebuild` 뒤로 가는 문제는
그대로 남습니다.

**단일 타깃.** 실행 타깃 하나에 전부 넣는 쪽입니다. 짧지만 위 경계가 전부
규율로 돌아갑니다. 이 프로젝트는 같은 문제를 코어 쪽에서 이미 한 번 풀었고
([C3](../03-core.md#c3--conversion-point)의 `vt` 모듈), 그때도 답은 모듈
경계였습니다.

## 결과

- **`swift build`는 cargo 산출물에 의존합니다.** SwiftPM은 cargo를 부를 줄
  모르므로 순서를 세우는 것은 조립 스크립트입니다. 순서를 건너뛰면 링크 오류가
  나므로 조용히 틀리지는 않습니다.
- **`unsafeFlags`로 정적 라이브러리 경로를 넘깁니다.** 그 결과 이 패키지는 다른
  패키지의 의존이 될 수 없습니다. 앱 전용 루트 패키지이므로 잃는 것이 없습니다.
- **`CKnotty`는 `include/knotty.h`를 그대로 가리킵니다.** 헤더는 M0 이후 동결
  상태이고 생성물이므로, Swift 쪽에 사본을 두면 그 사본이 두 번째 진실이 됩니다.
- **`swift run`으로는 앱이 제대로 뜨지 않습니다.** AppKit이 요구하는 것은 번들
  이고, metallib도 번들에 있습니다. 개발 중에도 실행 경로는 스크립트가 만든
  `.app` 하나입니다.
- **렌더러 골든은 아틀라스 좌표를 비교하지 않고, 셀 메트릭을 상수로 주입받습니다.**
  로컬이 macOS 26이고 CI 러너가 `macos-15`인데(Zig 0.15.2가 macOS 26 SDK에
  링크하지 못해 고정), 폰트 래스터 결과와 advance는 OS 버전 사이에서 보장되지
  않습니다. 비교 대상은 렌더러가 내리는 판단 — 셀마다 어느 글리프, 어느 색,
  커서 반전 결과 — 이고 아틀라스 패킹은 따로 봅니다.
