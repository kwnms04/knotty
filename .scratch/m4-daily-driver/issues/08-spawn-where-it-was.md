# 셸이 지정된 디렉터리에서 뜬다

Blocked by: 07

## What to build

세션을 만들 때 작업 디렉터리를 줄 수 있고, **셸에 아무 설정도 넣지 않은 상태에서**
스냅샷의 `pwd`가 채워집니다.

`kt_session_new_pty`에 작업 디렉터리 인자가 없어 지금은 프로세스 전역 cwd를
상속시킵니다. 창마다 다른 디렉터리를 요구하는 09가 그 방식으로는 서지 않습니다.
M2가 open-question으로 올린 결함이고 여기서 갚습니다.

**`pwd`는 기본 상태에서 오지 않습니다.** 그 값은 OSC 7에서 오는데, `/etc/zshrc`가
`/etc/zshrc_$TERM_PROGRAM`을 source하고 존재하는 파일은 `Apple_Terminal` 것
하나뿐입니다. 07이 `TERM_PROGRAM=knotty`를 달고 나면 zsh는 OSC 7을 내보내지
않습니다.

**`proc_pidinfo(PROC_PIDVNODEPATHINFO)`로 포어그라운드 프로세스의 것을 읽어
채웁니다.** 07이 이미 잡은 pid를 그대로 쓰므로 물어보는 자리도 같고, 스냅샷 필드가
이미 있으므로 이것 자체로는 ABI가 자라지 않습니다. cf.
[0020](../../../docs/adr/0020-restore-windows-ourselves.md)

## Acceptance criteria

- [x] 스폰 시 준 디렉터리에서 셸이 뜬다
- [x] 셸에 아무 설정이 없어도 `pwd`가 채워진다
- [x] OSC 7을 내보내는 셸에서는 그 값이 쓰인다
- [x] 자식이 디렉터리를 옮기면 `pwd`가 따라간다
- [x] tmux 안에서는 tmux를 띄운 자리가 나온다 — 문서화된 한계

넷은 `crates/knotty-ffi/tests/abi.rs`가 `/bin/sh`로 덮고, 다섯째는
`docs/06-integration.md`의 항목입니다. 자동 테스트가 닿지 못하는 것은 **앱이
실제로 띄우는 셸**이라, 넷 모두 `/bin/zsh -l`로 한 번 더 통과시켰습니다 —
ABI를 통해서이지 창을 통해서가 아닙니다. `/private/tmp`에서 스폰하면 `pwd`가
`/private/tmp`, 거기서 `cd /usr/lib` 하면 `/usr/lib`, 디렉터리를 주지 않으면
프로세스의 cwd. tmux는 `$HOME`에서 띄우고 그 안에서 `cd /usr/lib` 한 뒤에도
`/Users/kwn` — 한계가 적힌 그대로입니다. 셸에는 아무것도 넣지 않았고,
`/etc/zshrc_knotty`는 없습니다.

## 창으로 확인할 것

이 기계에는 접근성·화면기록 권한이 없어 GUI 동작은 검증하지 못했습니다. 이
변경이 든 워크트리는 지워졌으므로, 메인 체크아웃에서 새로 빌드해 엽니다 —
거기 남아 있는 옛 `build/knotty.app`은 이 변경이 없습니다.

```sh
cd /Users/kwn/work/knotty && git pull
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \
ZIG=/opt/homebrew/opt/zig@0.15/bin/zig \
ZIG_SYSROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk \
./scripts/build-app.sh
open /Users/kwn/work/knotty/build/knotty.app
```

- [ ] 창이 열리고 셸이 여느 때처럼 뜬다 — 스폰 인자가 하나 늘어난 것이 창을
      깨지 않았는지
- [ ] 창을 여럿 열어도 각자 살아 있다
- [ ] 실행 중인 창을 닫으면 여전히 경고가 뜨고, 조용한 창은 말없이 닫힌다 —
      07의 판정이 `libc::tcgetpgrp`으로 옮겨졌으므로

`pwd`를 창에서 눈으로 볼 자리는 아직 없습니다. 그것을 쓰는 것은 09이며,
지금 앱은 모든 창에 디렉터리를 주지 않습니다.
