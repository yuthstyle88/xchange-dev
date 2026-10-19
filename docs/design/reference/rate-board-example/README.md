# Reference: the example rate board (P01 v2)

**Where these files come from.** The owner shared `https://rate.xc-th.work/rate-board?branch=MAIN` on 2026-10-19 and said *"มันคือ ตัวอย่าง แต่เราต้องเอาเข้ามาใส่ใน xchange"* (xchange #564 issuecomment-6267770624). The FE lane controller claude/5edf7bb5 saved the page's stylesheet, script and body as served that day. These are byte copies:

| File | sha256 |
|---|---|
| `rate-board.css` | `26c0dc7f76b828047c78f062159d1b5c33e58f2663def26263418d5834cbc1de` |
| `rate-board.js` | `76d8afbb60cf44a5232a8bdb5b30de91517723f300f0b048a3521eb15c0c745a` |
| `rate-board.body.html` | `31bf20fb3c2eb80c0704ec8c060fa935a4f498cb4a71e9ec25d762e46d0414a8` |

**What they are for:** the look and the behaviour that `docs/design/blueprint/rate-display.md` §9 (P01 v2) brings into xchange. They are a reference, not source:
- nothing under `web/` imports them;
- no build or test reads them;
- they must not be copied into `web/` as they are.

§9.1 lists what v2 deliberately does differently: decimal strings instead of JS floats, no `:root` tokens, vendored flags instead of `/flags/` from that host, and the existing public rates route instead of `/api/rate-board`.

Do not edit these files; a newer example is a new copy with a new date.
