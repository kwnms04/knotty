# 창이 돌아온다

Blocked by: 06, 08

## What to build

껐다 켜면 종료 시점에 열려 있던 창들이 위치와 크기와 작업 디렉터리째 돌아옵니다.

**`UserDefaults`에 창마다 `(프레임, 작업 디렉터리)`를 직접 저장합니다.** macOS 상태
복원을 쓰지 않는 이유는 `talagentd`가 복원 가능한 창의 내용이 바뀔 때마다 스냅샷을
다시 뜨기 때문이고 — 터미널은 내용이 끊임없이 바뀌는 창입니다 — 우리는 스크롤백을
복원하지 않으므로 그 기계가 필요한 이유도 없습니다. cf.
[0020](../../../docs/adr/0020-restore-windows-ourselves.md)

**저장은 사건에 겁니다** — 창 이동·리사이즈·개폐와 `pwd` 변화. 주기적 저장은
B7("유휴 = 출력 없음 + 입력 없음")을 깨는데, 그 조건은 커서 깜빡임까지 버려 가며
지킨 것입니다.

**복원은 프레임대로 열되, 어떤 `NSScreen`과도 겹치지 않으면 주 화면 중앙에
엽니다.** 프레임이 진실이고 그리드는 프레임에서 유도되므로, 앱이 꺼진 사이 폰트가
바뀌었으면 창 크기가 유지되고 그리드가 달라집니다.

`NSQuitAlwaysKeepsWindows`를 읽어 존중합니다.

## Acceptance criteria

- [ ] ⌘Q 뒤 다시 켜면 창들이 위치와 크기로 돌아온다 — **절반만**
- [x] 각 창의 셸이 저장된 디렉터리에서 뜬다
- [ ] 명시적으로 닫은 창은 돌아오지 않는다
- [x] 강제 종료 뒤에도 마지막 상태가 남아 있다
- [x] 겹치는 화면이 없는 프레임은 주 화면 중앙에 열린다
- [x] `NSQuitAlwaysKeepsWindows`가 꺼져 있으면 복원하지 않는다
- [x] 유휴 상태에서 저장이 돌지 않는다

이 기계에는 접근성·화면기록 권한이 없습니다. 그래서 **창을 밖에서 몰았습니다**
— 스토어를 `defaults write`로 심고, 앱을 띄우고, 앱이 도로 적은 것과 자식
셸의 cwd를 읽는 방식입니다. 키를 누르지 않고 닿는 데까지가 위의 넷입니다.

- 프레임 `{{140, 220}, {721, 463}}`을 심고 띄우니 앱이 **똑같은 프레임을** 도로
  적었습니다. 창이 저장된 자리에 그대로 열렸다는 뜻입니다.
- 창 둘을 `/usr/share`와 `/private/etc`로 심으니 자식 zsh 둘의 cwd가
  각각 그것이었습니다 (`lsof -a -p <pid> -d cwd`).
- `kill -9` 뒤에도 스토어는 그대로였습니다. `UserDefaults`는 cfprefsd가 들고
  있어 우리 프로세스가 죽는 것과 무관합니다.
- 어떤 화면에도 없는 `{{9000, 9000}, {640, 400}}`은 `{{580, 365}, {640, 400}}`로
  돌아왔습니다 — 1800pt 폭 주 화면의 한가운데입니다.
- `NSQuitAlwaysKeepsWindows`를 `false`로 두니 심어 둔 창 둘을 무시하고 `$HOME`에
  새 창 하나를 열었습니다.

유휴 저장은 **호출부를 세어 확인했습니다**. 저장을 부르는 곳은 넷뿐이고
(`open`, `windowWillClose`, `windowMovedOrResized`, 세션의 `onWorkingDirectory`)
모두 사건 처리기입니다. 복원 경로에 타이머는 없습니다 — 앱 타깃에 남은 타이머는
드래그 중에만 사는 autoscroll과 M3의 디스플레이 링크뿐입니다. 측정이 아니라
읽어서 확인한 것입니다.

`⌘Q` 쪽 절반은 닿지 못했습니다. **복원**은 위에서 확인했지만 **⌘Q가 남기는
것**은 키를 눌러야 합니다. `applicationWillTerminate(_:)`이 창들의
`willCloseNotification`보다 **먼저** 온다는 것은 단독 프로브로 재어 두었고
(그래서 종료 중에는 저장을 멈춥니다), 그 가드가 실제 ⌘Q에서 도는 것은 아직
눈으로 볼 일입니다.

## 창으로 확인할 것

워크트리는 지워졌으므로 메인 체크아웃에서 새로 빌드해 엽니다.

```sh
cd /Users/kwn/work/knotty && git pull
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \
ZIG=/opt/homebrew/opt/zig@0.15/bin/zig \
ZIG_SYSROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk \
./scripts/build-app.sh
open /Users/kwn/work/knotty/build/knotty.app
```

- [ ] 창 둘을 서로 다른 자리로 끌어 두고 각각에서 `cd` 한 뒤 ⌘Q → 다시 열면
      자리도 크기도 디렉터리도 그대로 (`defaults read com.github.kwnms04.knotty
      windows`로 스토어도 함께 볼 수 있습니다)
- [ ] 창 셋 중 하나를 ⌘W로 닫고 ⌘Q → 다시 열면 둘만 돌아온다
- [ ] 창을 끄는 동안 자리가 따라 저장된다 — 드래그를 놓고 위 `defaults read`

`cd`가 저장을 부르는 것은 절반만 확인했습니다. 스폰 직후의 `pwd`가 스토어에
들어가는 것은 위에서 봤지만(앱이 `/usr/lib`을 도로 적었습니다), 창 안에서 손으로
`cd` 한 뒤에 갱신되는지는 키를 눌러야 합니다.
