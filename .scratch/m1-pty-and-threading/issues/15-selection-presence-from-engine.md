# 선택 존재 여부를 세션이 아니라 엔진에 묻는다

Blocked by: —
Was: #29

## What to build

세션이 들고 있는 선택 기록(`Session::selection_screen`)을 지우고, 선택이
존재하는지를 엔진에 직접 묻습니다.

이 기록이 있던 이유는 [0012](https://github.com/kwnms04/knotty/blob/main/docs/adr/0012-own-the-binding-layer.md)가
걷어낸 안전 래퍼였습니다 — C API에는 질의가 있는데 래퍼가 원시 핸들을 감춰
닿지 않았고, 그래서 knotty가 "선택을 건 화면"을 따로 적어 두고 활성 화면과
비교해 왔습니다. 파사드가 `-sys`에 직접 닿으면서 `GHOSTTY_TERMINAL_DATA_SELECTION`이
손에 들어왔습니다. 선택이 없으면 `GHOSTTY_NO_VALUE`로 답합니다.

**관측 가능한 동작이 하나 바뀝니다.** 지금은 터미널을 통째로 리셋하는 시퀀스가
엔진의 선택을 버려도 knotty의 기록은 남아, 그 뒤 잠시 `has_selection`이 참으로
읽힙니다. `03-core.md` C3가 이 어긋남을 적어 두고 있습니다. 엔진에 직접 물으면
사라집니다.

파사드(#27)와 같은 커밋에 넣지 않은 이유는 #27의 검증 기준이 "골든 넷이 갱신
없이 통과"였기 때문입니다. 이건 동작이 바뀌는 변경이라 자기 테스트를 데리고
와야 합니다.

## Acceptance criteria

- [x] `Session::selection_screen`과 `vt::Screen`이 사라진다
- [x] `has_selection`이 엔진의 답을 그대로 옮긴다
- [x] 리셋 뒤 `has_selection`이 거짓임을 잠그는 테스트가 생긴다 — 지금은 없다
- [x] `abi.rs`의 기존 선택 테스트 다섯이 갱신 없이 통과한다
- [x] 골든 넷이 갱신 없이 통과한다
- [x] `03-core.md` C3에서 이 우회로에 대한 서술이 사라진다
