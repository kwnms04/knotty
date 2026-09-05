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

- [ ] 스폰 시 준 디렉터리에서 셸이 뜬다
- [ ] 셸에 아무 설정이 없어도 `pwd`가 채워진다
- [ ] OSC 7을 내보내는 셸에서는 그 값이 쓰인다
- [ ] 자식이 디렉터리를 옮기면 `pwd`가 따라간다
- [ ] tmux 안에서는 tmux를 띄운 자리가 나온다 — 문서화된 한계
