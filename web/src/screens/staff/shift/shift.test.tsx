// S-5 #578: the staff shift card (my shift: open / close count and hand-over; counts to verify; hand-overs to me).
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order, the owner having ordered it on 2026-10-11). Do not edit, delete or weaken; add tests in a
// file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md (SD)
//   §4      shift/: my shift (my_shift): open count, close count, hand-over to a teller (countSubmit, handoverStart);
//           counts waiting for me to verify (cash_counts, countVerify; ShiftCloseCertify); hand-overs to me
//           (handovers_pending). Texts staff.shift.*.
//   §6.1    B6 can(ShiftOperate) and clocked in: the open count when I have no OPEN shift, close and hand-over when I
//           do. B7 open/close counts to verify with can(ShiftCloseCertify), never my own count; a hand-over whose
//           receiver I am: accept or reject it, never through countVerify (which refuses a HANDOVER_OUT count).
//   §6.2    C7 countVerify {id}; C7b handoverAccept / handoverReject {id} (no reason). After success: notice
//           "บันทึกแล้ว", staff_floor and the card's lists re-read; on refusal t('error.'+code), nothing re-read.
//   S2 no role literal · S5 nothing through /api/staff/rpc · S8 TH/EN · §8.1 no Number-constructor call in staff/**.
// CA-8 #624 (the server half, cash-shift-transfer.md §4.4, §4.7; its provided tests @ 630b95f3 and its rulings):
//   routes  POST /api/cash-counts/{id}/verify · POST /api/handovers/{id}/accept · POST /api/handovers/{id}/reject
//   views   cash_counts (paged, filter status WAITING | VERIFIED), handovers_pending, my_shift; rows are the design's
//           records camelCased (#624 G1). Since #624 merged, the fixtures are typed by its generated files:
//           CashCountRow (cash_counts.ts), HandoverPendingRow (handovers_pending.ts), DenominationRow, HandoverReceiverRow, MyShift.
//   types   commands in web/src/shared/api/generated/cash.ts; each view's row and filter types in its own file
//           (cash_counts.ts, handovers_pending.ts, my_shift.ts; BE ruling #624 issuecomment-6107777140).
//
// S9 count rows (BE folded the view into #624, issuecomment-6107796305; FE rulings on #578, 2026-10-11):
//   view    GET /api/v3/views/denominations → data { rows: [{ id, currency, faceValue: string, groupCode }] }, ordered
//           by currency, then face value, then id; TS in generated/denominations.ts. `id` is the denominationId a count
//           line carries. Same permission as cash_counts. BE ruling #624 issuecomment-6107853376: the view excludes the
//           catch-all (face_value NULL) rows, so the screen never shows one.
//   rows    one row per face value, its faceValue shown exactly as the wire string.
//   C5      countSubmit {kind: OPEN | CLOSE, lines: [{currency, denominationId, qty: str}]}: a typed whole-number
//           quantity, as a string, for each face value typed; a row left empty is not sent.
//   C6      handoverStart POST /api/handovers {fromShiftId, receiverId, lines} (#624's shape, FE ruling over SD C6's
//           older one): fromShiftId = staff_floor.myShift.id; receiverId = the userId picked from the view
//           handover_receivers (BE ruling #624 issuecomment-6107853376): GET /api/v3/views/handover_receivers → data
//           { rows: [{ userId, name }] }, the session branch's active ShiftOperate holders without the caller, ordered
//           by name then userId, no clocked-in filter; TS in generated/handover_receivers.ts.
//           Hooks: [data-testid=shift-handover] opens [data-testid=shift-handover-form], a <form> with a
//           <select data-testid=shift-receiver> (one option per row, value = userId, in the server's order; an
//           empty-value placeholder is allowed), the face-value rows, and [data-testid=shift-handover-save], disabled
//           while no receiver can be picked.
//
// The contract this test assumes ([required] unless marked):
//   shift/card.ts     export const card: StaffCard; id = 'shift'; visible reads only can(ctx.me, …) and ctx.floor:
//                     shown with can(ShiftOperate) or can(ShiftCloseCertify).
//   shift/i18n.ts     export const th / en with the same keys, no zh. Keys read here: openCount closeCount handover
//                     verify accept reject saved.
//   DOM hooks         [data-testid=shift-open-count] / [data-testid=shift-close-count] / [data-testid=shift-handover]
//                     the B6 controls (a button or a form that opens the count); absent when B6 hides them.
//                     [data-count-id="<id>"] one element per count listed to verify, with a
//                     [data-testid=shift-verify] button inside when I may verify it.
//                     [data-handover-id="<id>"] one element per pending hand-over to me, with
//                     [data-testid=shift-accept] and [data-testid=shift-reject] buttons inside.
//                     [data-testid=shift-notice] / [data-testid=shift-error] the after-write lines.
//   Reads             the card reads cash_counts with can(ShiftCloseCertify) and handovers_pending with
//                     can(ShiftOperate) through ApiClient.view (GET /api/v3/views/<name>). The stub below answers
//                     every query of a view with the same rows, so the card decides what to show from the rows.
//   Bodies            a `branch` member, if sent, must be the actor's own branch; nothing else beyond the design's.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { CashCountRow } from "../../../shared/api/generated/cash_counts";
import type { HandoverPendingRow } from "../../../shared/api/generated/handovers_pending";
import type { DenominationRow } from "../../../shared/api/generated/denominations";
import type { HandoverReceiverRow } from "../../../shared/api/generated/handover_receivers";
import type { MyShift } from "../../../shared/api/generated/my_shift";
import type { StaffCard, StaffCtx } from "../cards";

type Texts = {
  th: Record<string, string>;
  en: Record<string, string>;
} & Record<string, unknown>;

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");
const textMods = import.meta.glob<Texts>("./i18n.ts", { eager: true });
const shiftSources = import.meta.glob<string>(
  ["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"],
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
);
const staffSources = import.meta.glob<string>(["../**/*.ts", "../**/*.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/shift/card.ts exists").toBeDefined();
  return (await l!()).card;
}

function texts(): Texts {
  const m = textMods["./i18n.ts"];
  expect(m, "web/src/screens/staff/shift/i18n.ts exists").toBeDefined();
  return m!;
}

const KEYS = [
  "openCount",
  "closeCount",
  "handover",
  "verify",
  "accept",
  "reject",
  "saved",
] as const;

function th(key: (typeof KEYS)[number]): string {
  const v = texts().th[key];
  expect(v, `shift th['${key}']`).toBeTypeOf("string");
  return v!;
}

// ── fixtures ────────────────────────────────────────────────────────────────────────────────

const BRANCH = "MAIN";
const ME = "00000000-0000-4000-8000-0000000000a1";
const OTHER = "00000000-0000-4000-8000-0000000000a2";
const THIRD = "00000000-0000-4000-8000-0000000000a3";

function me(permissions: string[]): StaffMeView {
  return {
    userId: ME,
    name: "Ploy",
    roles: [],
    branchId: BRANCH,
    branchName: "XC-B",
    permissions,
  };
}

function floor(over: Partial<StaffFloorView> = {}): StaffFloorView {
  return {
    phase: "OPEN",
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  };
}

const CLOCKED_IN = "2026-10-11T01:00:00Z";
const MY_SHIFT = {
  id: "11111111-1111-4111-8111-111111111111",
  openedAt: "2026-10-11T01:05:00Z",
};
/** The `my_shift` view of that shift (#624 `my_shift.ts`). */
const MY_SHIFT_VIEW: MyShift = {
  id: MY_SHIFT.id,
  status: "OPEN",
  openedAt: MY_SHIFT.openedAt,
  balances: [],
};

type Count = CashCountRow;

function count(
  id: string,
  kind: string,
  countedBy: string,
  verifiedBy: string | null = null,
): Count {
  return {
    id,
    branchId: BRANCH,
    shiftId: "22222222-2222-4222-8222-222222222222",
    kind,
    countedBy,
    countedAt: "2026-10-11T02:00:00Z",
    verifiedBy,
    verifiedAt: verifiedBy ? "2026-10-11T02:10:00Z" : null,
    note: null,
    totals: [],
  };
}

type Handover = HandoverPendingRow;

function handover(id: string, receiverId: string): Handover {
  return {
    id,
    fromShiftId: "33333333-3333-4333-8333-333333333333",
    countId: "44444444-4444-4444-8444-444444444444",
    receiverId,
    status: "PENDING",
    createdAt: "2026-10-11T03:00:00Z",
  };
}

const C_OPEN = "c0000000-0000-4000-8000-000000000001"; // another teller's OPEN count, waiting
const C_CLOSE = "c0000000-0000-4000-8000-000000000002"; // another teller's CLOSE count, waiting
const C_MINE = "c0000000-0000-4000-8000-000000000003"; // my own CLOSE count, waiting
const C_DONE = "c0000000-0000-4000-8000-000000000004"; // already verified
const C_HAND = "c0000000-0000-4000-8000-000000000005"; // a HANDOVER_OUT count, waiting
const H_MINE = "d0000000-0000-4000-8000-000000000001"; // a hand-over to me
const H_OTHER = "d0000000-0000-4000-8000-000000000002"; // a hand-over to someone else

const COUNTS: Count[] = [
  count(C_OPEN, "OPEN", OTHER),
  count(C_CLOSE, "CLOSE", THIRD),
  count(C_MINE, "CLOSE", ME),
  count(C_DONE, "OPEN", OTHER, THIRD),
  count(C_HAND, "HANDOVER_OUT", OTHER),
];
const HANDOVERS: Handover[] = [handover(H_MINE, ME), handover(H_OTHER, THIRD)];

/** The denominations view's rows, as #624 orders them: currency, then faceValue nulls first, then id. */
const DENOMS: DenominationRow[] = [
  { id: 12, currency: "THB", faceValue: "100.0000", groupCode: "NOTE" },
  { id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" },
  { id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" },
];

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};
type Answer = { status: number; body: unknown } | "network";

let sent: Sent[];
let reads: string[];
let answers: Record<string, Answer[]>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = {
    cash_counts: COUNTS,
    handovers_pending: HANDOVERS,
    my_shift: null,
    denominations: { rows: DENOMS },
    handover_receivers: { rows: [] },
    ...views,
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split("?")[0]!;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "GET") {
        const name = path.startsWith("/api/v3/views/")
          ? path.slice("/api/v3/views/".length)
          : "";
        reads.push(name || path);
        if (name in all)
          return json(200, { version: 1, changed: true, data: all[name] });
        if (path === "/health/ready") {
          return json(200, {
            version: "3.0.0",
            build: "abc1234",
            channel: "staging",
            builtAt: "2026-10-11T00:00:00Z",
          });
        }
        return json(404, { error: { code: "notFound", message: "not found" } });
      }
      const headers = new Headers(init?.headers);
      sent.push({
        url: path,
        method,
        key: headers.get("idempotency-key"),
        body: init?.body as string | undefined,
      });
      const next = answers[path]?.shift();
      if (next === "network") throw new TypeError("Failed to fetch");
      if (next) return json(next.status, next.body);
      return json(200, { id: path.split("/")[3], status: "ACCEPTED" });
    }),
  );
}

function ctxOf(m: StaffMeView, f: StaffFloorView | undefined) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: f, refresh };
  return { ctx, refresh };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView, f: StaffFloorView | undefined) {
  const card = await loadCard();
  const { ctx, refresh } = ctxOf(m, f);
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
}

function unmount() {
  dispose?.();
  dispose = undefined;
  host.innerHTML = "";
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

/**
 * The body a command sent, as an object, compared on what it says: #624's generated inputs (`cash.ts`) carry
 * `branch: string | null` and other optional members as `null`, so a `branch` member (the actor's own branch, or
 * null) and every member whose value is null are left out before comparing. Merging main after #624 (2026-10-11).
 */
function bodyOf(s: Sent): Record<string, unknown> {
  expect(s.body, `${s.url} has a JSON body`).toBeTypeOf("string");
  const b = JSON.parse(s.body!) as Record<string, unknown>;
  if ("branch" in b) {
    expect(
      [BRANCH, null],
      "a branch member names the actor's own branch, or null",
    ).toContain(b["branch"]);
    delete b["branch"];
  }
  for (const k of Object.keys(b)) if (b[k] === null) delete b[k];
  return b;
}

function verifyOf(id: string): HTMLElement | null {
  return q(`[data-count-id="${id}"] ${tid("shift-verify")}`);
}

beforeEach(() => {
  sent = [];
  reads = [];
  answers = {};
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  unmount();
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

// ── the card record and its texts ───────────────────────────────────────────────────────────

describe("S-5 the card record (SD:§4)", () => {
  it('exports card { id: "shift", order, visible, Component }', async () => {
    const card = await loadCard();
    expect(card.id).toBe("shift");
    expect(Number.isFinite(card.order)).toBe(true);
    expect(card.visible).toBeTypeOf("function");
    expect(card.Component).toBeTypeOf("function");
  });

  it("shift/i18n.ts has th and en with the same keys (every key this test reads), and no zh table (S8)", () => {
    const t = texts();
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const k of KEYS) {
      expect(t.th[k], `th['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}'] is English, not the Thai copied`).not.toBe(
        t.th[k],
      );
    }
    expect(th("saved")).toBe("บันทึกแล้ว");
    expect("zh" in t, "staff screens are TH/EN only").toBe(false);
  });

  // The generated files are imported above as types (CA-8 #624 merged): a missing or renamed one is a type error.
  it("the card imports its commands from cash.ts and its rows from each view's file", () => {
    const code = Object.values(shiftSources).join("\n");
    expect(
      Object.keys(shiftSources).length,
      "shift/ has code besides its tests",
    ).toBeGreaterThan(0);
    for (const f of [
      "cash",
      "cash_counts",
      "handovers_pending",
      "denominations",
      "handover_receivers",
    ]) {
      expect(
        new RegExp(
          `from\\s+['"](\\.\\./){3}shared/api/generated/${f}['"]`,
        ).test(code),
        `a shift/ source imports ../../../shared/api/generated/${f}`,
      ).toBe(true);
    }
  });
});

// ── B6 / B7: who sees what ──────────────────────────────────────────────────────────────────

describe("the card is shown with can(ShiftOperate) or can(ShiftCloseCertify)", () => {
  it("visible exactly when the actor holds ShiftOperate or ShiftCloseCertify", async () => {
    const card = await loadCard();
    const v = (perms: string[], f?: StaffFloorView) =>
      card.visible(ctxOf(me(perms), f).ctx);
    expect(v(["ShiftOperate"], floor())).toBe(true);
    expect(v(["ShiftCloseCertify"], floor())).toBe(true);
    expect(v(["ShiftOperate"])).toBe(true);
    expect(v([], floor())).toBe(false);
    expect(
      v(
        ["AttendanceClock", "CashManage", "BranchDayOperate", "CompatRead"],
        floor(),
      ),
    ).toBe(false);
  });
});

describe("B6 my shift: ShiftOperate and clocked in; open with no OPEN shift, close and hand-over with one", () => {
  it("clocked in with no shift: the open count only", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    expect(must(tid("shift-open-count")).textContent).toContain(
      th("openCount"),
    );
    expect(q(tid("shift-close-count"))).toBeNull();
    expect(q(tid("shift-handover"))).toBeNull();
  });

  it("clocked in with my OPEN shift: close and hand-over, not open", async () => {
    stub({ my_shift: MY_SHIFT_VIEW });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    expect(q(tid("shift-open-count"))).toBeNull();
    expect(must(tid("shift-close-count")).textContent).toContain(
      th("closeCount"),
    );
    expect(must(tid("shift-handover")).textContent).toContain(th("handover"));
  });

  it("not clocked in: none of the three, with or without a shift", async () => {
    for (const myShift of [null, MY_SHIFT]) {
      stub();
      await mount(me(["ShiftOperate"]), floor({ myClockIn: null, myShift }));
      expect(
        q(tid("shift-open-count")),
        `shift ${myShift ? "open" : "none"}`,
      ).toBeNull();
      expect(q(tid("shift-close-count"))).toBeNull();
      expect(q(tid("shift-handover"))).toBeNull();
      unmount();
    }
  });

  it("without ShiftOperate: none of the three, even clocked in", async () => {
    for (const myShift of [null, MY_SHIFT]) {
      stub();
      await mount(
        me(["ShiftCloseCertify", "AttendanceClock"]),
        floor({ myClockIn: CLOCKED_IN, myShift }),
      );
      expect(q(tid("shift-open-count"))).toBeNull();
      expect(q(tid("shift-close-count"))).toBeNull();
      expect(q(tid("shift-handover"))).toBeNull();
      unmount();
    }
  });
});

describe("B7 counts to verify: ShiftCloseCertify, open or close counts still waiting, never my own", () => {
  it("lists a verify button for another person's waiting OPEN and CLOSE counts", async () => {
    stub();
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    expect(verifyOf(C_CLOSE)).not.toBeNull();
    expect(verifyOf(C_OPEN)!.textContent).toContain(th("verify"));
  });

  it("no verify button for my own count, a verified count, or a HANDOVER_OUT count", async () => {
    stub();
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    expect(verifyOf(C_MINE), "never my own count (C7)").toBeNull();
    expect(verifyOf(C_DONE), "already verified").toBeNull();
    expect(
      verifyOf(C_HAND),
      "a hand-over count is never verified through countVerify (B7)",
    ).toBeNull();
  });

  it("without ShiftCloseCertify there is no verify button, and cash_counts is not needed for it", async () => {
    stub();
    await mount(me(["ShiftOperate"]), floor({ myClockIn: CLOCKED_IN }));
    expect(host.querySelector(tid("shift-verify"))).toBeNull();
  });
});

describe("B7 hand-overs to me: accept or reject, only where I am the receiver", () => {
  it("a pending hand-over to me has accept and reject; one to someone else has neither", async () => {
    stub();
    await mount(me(["ShiftOperate"]), floor({ myClockIn: CLOCKED_IN }));
    await vi.waitFor(() =>
      expect(
        q(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`),
      ).not.toBeNull(),
    );
    expect(
      must(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`).textContent,
    ).toContain(th("accept"));
    expect(
      must(`[data-handover-id="${H_MINE}"] ${tid("shift-reject")}`).textContent,
    ).toContain(th("reject"));
    expect(
      q(`[data-handover-id="${H_OTHER}"] ${tid("shift-accept")}`),
    ).toBeNull();
    expect(
      q(`[data-handover-id="${H_OTHER}"] ${tid("shift-reject")}`),
    ).toBeNull();
  });

  it("without ShiftOperate there is no accept or reject", async () => {
    stub();
    await mount(me(["ShiftCloseCertify"]), floor());
    expect(host.querySelector(tid("shift-accept"))).toBeNull();
    expect(host.querySelector(tid("shift-reject"))).toBeNull();
  });
});

// ── C7 / C7b: what each control sends ───────────────────────────────────────────────────────

describe("C7 and C7b each control sends exactly its command and body", () => {
  it("C7 verify sends POST /api/cash-counts/{id}/verify {id}", async () => {
    stub();
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_CLOSE)).not.toBeNull());
    await click(verifyOf(C_CLOSE)!);
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/cash-counts/${C_CLOSE}/verify`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ id: C_CLOSE });
    expect(sent[0]!.key, "an Idempotency-Key").toBeTruthy();
  });

  it("C7b accept sends POST /api/handovers/{id}/accept {id}", async () => {
    stub();
    await mount(me(["ShiftOperate"]), floor({ myClockIn: CLOCKED_IN }));
    await vi.waitFor(() =>
      expect(
        q(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`),
      ).not.toBeNull(),
    );
    await click(must(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`));
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/handovers/${H_MINE}/accept`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ id: H_MINE });
  });

  it("C7b reject sends POST /api/handovers/{id}/reject {id}, with no reason asked or sent", async () => {
    stub();
    await mount(me(["ShiftOperate"]), floor({ myClockIn: CLOCKED_IN }));
    await vi.waitFor(() =>
      expect(
        q(`[data-handover-id="${H_MINE}"] ${tid("shift-reject")}`),
      ).not.toBeNull(),
    );
    await click(must(`[data-handover-id="${H_MINE}"] ${tid("shift-reject")}`));
    expect(
      host.querySelector("dialog"),
      "no reason dialog: a hand-over has no reason column",
    ).toBeNull();
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/handovers/${H_MINE}/reject`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ id: H_MINE });
  });
});

// ── C5: a count is one row per face value, quantities as strings (S9) ───────────────────────

/** The count form: opened from the open- or close-count control. */
async function openCountForm(
  control: "shift-open-count" | "shift-close-count",
): Promise<HTMLFormElement> {
  await click(must(tid(control)));
  await vi.waitFor(() => expect(q(tid("shift-count-form"))).not.toBeNull());
  const form = must<HTMLFormElement>(tid("shift-count-form"));
  expect(form.tagName).toBe("FORM");
  return form;
}

function qtyInput(form: HTMLFormElement, id: number): HTMLInputElement {
  const i = form.querySelector<HTMLInputElement>(
    `input[data-denomination-id="${id}"]`,
  );
  expect(i, `a quantity input for denomination ${id}`).not.toBeNull();
  return i!;
}

function typeInto(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

async function saveCount(form: HTMLFormElement) {
  const save = form.querySelector<HTMLButtonElement>(tid("shift-count-save"));
  expect(save, `${tid("shift-count-save")} inside the form`).not.toBeNull();
  save!.click();
  await settle();
}

/** A body's lines, sorted by denominationId so the order the card sends them in is not pinned. */
function linesOf(body: Record<string, unknown>): unknown[] {
  return [...(body["lines"] as { denominationId: number }[])].sort(
    (a, b) => a.denominationId - b.denominationId,
  );
}

describe("S9 the count form: one row per face value from the denominations view", () => {
  it("a quantity input per face value, each labelled with its faceValue exactly as the wire string", async () => {
    stub();
    const form = await (async () => {
      await mount(
        me(["ShiftOperate"]),
        floor({ myClockIn: CLOCKED_IN, myShift: null }),
      );
      return openCountForm("shift-open-count");
    })();
    for (const d of DENOMS) {
      const input = qtyInput(form, d.id);
      const row =
        input.closest("[data-denomination-row]") ?? input.parentElement!;
      expect(row.textContent, `denomination ${d.id}`).toContain(d.faceValue);
      expect(row.textContent, `denomination ${d.id}`).toContain(d.currency);
    }
  });

  it("no input in the count form asks for an amount: every input of a face value row is its quantity", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    const form = await openCountForm("shift-open-count");
    const faceIds = new Set(DENOMS.map((d) => String(d.id)));
    for (const input of form.querySelectorAll<HTMLInputElement>("input")) {
      const id = input.getAttribute("data-denomination-id");
      expect(id, "every input names its denomination").not.toBeNull();
      expect(faceIds.has(id!), `input for a known denomination ${id}`).toBe(
        true,
      );
    }
  });
});

describe("C5 open and close counts send countSubmit with one line per typed face value", () => {
  it("open count: POST /api/cash-counts {kind: OPEN, lines: [{currency, denominationId, qty: str}]}", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    const form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "2");
    typeInto(qtyInput(form, 12), "3");
    await saveCount(form);
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/cash-counts",
    ]);
    const body = bodyOf(sent[0]!);
    expect(Object.keys(body).sort()).toEqual(["kind", "lines"]);
    expect(body["kind"]).toBe("OPEN");
    expect(linesOf(body)).toEqual([
      { currency: "THB", denominationId: 11, qty: "2" },
      { currency: "THB", denominationId: 12, qty: "3" },
    ]);
  });

  it("close count: kind CLOSE, and a face value left empty is not sent", async () => {
    stub({ my_shift: MY_SHIFT_VIEW });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    const form = await openCountForm("shift-close-count");
    typeInto(qtyInput(form, 21), "4");
    await saveCount(form);
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/cash-counts",
    ]);
    const body = bodyOf(sent[0]!);
    expect(body["kind"]).toBe("CLOSE");
    expect(body["shiftId"], "#624: a CLOSE count names the closing shift").toBe(
      MY_SHIFT.id,
    );
    expect(linesOf(body)).toEqual([
      { currency: "USD", denominationId: 21, qty: "4" },
    ]);
  });

  it("a quantity is sent as the typed string, never as a JSON number", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    const form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "10");
    await saveCount(form);
    expect(sent[0]!.body).toContain('"qty":"10"');
    expect(sent[0]!.body).not.toMatch(/"qty":\s*10\b/);
  });

  it("a refused count keeps the typed quantities and re-reads nothing", async () => {
    stub();
    answers["/api/cash-counts"] = [
      {
        status: 400,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    const form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "2");
    const before = reads.length;
    await saveCount(form);
    expect(must(tid("shift-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length).toBe(before);
    expect(
      qtyInput(must<HTMLFormElement>(tid("shift-count-form")), 11).value,
    ).toBe("2");
  });

  it("a retry after a network error resends the same key and a byte-equal body", async () => {
    stub();
    answers["/api/cash-counts"] = [
      "network",
      { status: 200, body: { id: "c1", status: "WAITING" } },
    ];
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    const form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "2");
    await saveCount(form);
    await saveCount(must<HTMLFormElement>(tid("shift-count-form")));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
  });
});

// ── C6: hand over to a teller (handover_receivers, #624 issuecomment-6107853376) ─────────────

/** handover_receivers as the server orders them (name, then userId); the caller is in it only to prove it is never offered. */
const RECEIVERS: HandoverReceiverRow[] = [
  { userId: THIRD, name: "Aom" },
  { userId: ME, name: "Ploy" },
  { userId: OTHER, name: "Somchai" },
];

async function openHandover(): Promise<HTMLFormElement> {
  await click(must(tid("shift-handover")));
  await vi.waitFor(() => expect(q(tid("shift-handover-form"))).not.toBeNull());
  const form = must<HTMLFormElement>(tid("shift-handover-form"));
  expect(form.tagName).toBe("FORM");
  await vi.waitFor(() =>
    expect(form.querySelector("input[data-denomination-id]")).not.toBeNull(),
  );
  return form;
}

function receiverIds(): string[] {
  return [...must<HTMLSelectElement>(tid("shift-receiver")).options]
    .map((o) => o.value)
    .filter((v) => v !== "");
}

describe("C6 hand over: the receiver is picked from handover_receivers; the body is #624's", () => {
  it("the picker lists the rows in the server's order, each option its userId and name, and never the caller", async () => {
    stub({ handover_receivers: { rows: RECEIVERS } });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    await openHandover();
    await vi.waitFor(() => expect(receiverIds().length).toBeGreaterThan(0));
    expect(receiverIds()).toEqual([THIRD, OTHER]);
    const select = must<HTMLSelectElement>(tid("shift-receiver"));
    expect(select.tagName).toBe("SELECT");
    const aom = [...select.options].find((o) => o.value === THIRD)!;
    expect(aom.textContent).toContain("Aom");
    expect(receiverIds(), "the caller is never offered").not.toContain(ME);
  });

  it("start sends POST /api/handovers {fromShiftId: myShift.id, receiverId, lines: [{currency, denominationId, qty: str}]}", async () => {
    stub({ handover_receivers: { rows: RECEIVERS } });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    const form = await openHandover();
    await vi.waitFor(() => expect(receiverIds().length).toBeGreaterThan(0));
    typeInto(
      must<HTMLSelectElement>(
        tid("shift-receiver"),
      ) as unknown as HTMLInputElement,
      OTHER,
    );
    typeInto(qtyInput(form, 11), "2");
    await settle();
    must<HTMLButtonElement>(tid("shift-handover-save")).click();
    await settle();
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/handovers",
    ]);
    const body = bodyOf(sent[0]!);
    expect(Object.keys(body).sort()).toEqual([
      "fromShiftId",
      "lines",
      "receiverId",
    ]);
    expect(body["fromShiftId"]).toBe(MY_SHIFT.id);
    expect(body["receiverId"]).toBe(OTHER);
    expect(linesOf(body)).toEqual([
      { currency: "THB", denominationId: 11, qty: "2" },
    ]);
  });

  it("no receivers: none can be picked, and start stays disabled", async () => {
    stub({ handover_receivers: { rows: [] } });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    const form = await openHandover();
    expect(receiverIds()).toEqual([]);
    typeInto(qtyInput(form, 11), "2");
    await settle();
    const start = must<HTMLButtonElement>(tid("shift-handover-save"));
    expect(start.disabled, "start is disabled with no receiver").toBe(true);
    start.click();
    await settle();
    expect(sent, "nothing is sent").toEqual([]);
  });

  it("only the caller in the rows also leaves nothing to pick, and start disabled", async () => {
    stub({ handover_receivers: { rows: [{ userId: ME, name: "Ploy" }] } });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    await openHandover();
    await settle();
    expect(receiverIds()).toEqual([]);
    expect(must<HTMLButtonElement>(tid("shift-handover-save")).disabled).toBe(
      true,
    );
  });
});

// ── after a write ───────────────────────────────────────────────────────────────────────────

describe("SD §6.2 after a write: success, refusal, retry", () => {
  it("success: the notice is t(saved), staff_floor is re-read (ctx.refresh) and the card's list is re-read", async () => {
    stub();
    const { refresh } = await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    const before = reads.filter((r) => r === "cash_counts").length;
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() =>
      expect(must(tid("shift-notice")).textContent).toBe(th("saved")),
    );
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(
      reads.filter((r) => r === "cash_counts").length,
      "cash_counts is read again",
    ).toBeGreaterThan(before);
  });

  it("success on a hand-over re-reads handovers_pending", async () => {
    stub();
    const { refresh } = await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN }),
    );
    await vi.waitFor(() =>
      expect(
        q(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`),
      ).not.toBeNull(),
    );
    const before = reads.filter((r) => r === "handovers_pending").length;
    await click(must(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`));
    await vi.waitFor(() =>
      expect(must(tid("shift-notice")).textContent).toBe(th("saved")),
    );
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(
      reads.filter((r) => r === "handovers_pending").length,
    ).toBeGreaterThan(before);
  });

  it("a refusal shows t('error.'+code) and re-reads nothing", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      {
        status: 403,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    const before = reads.length;
    await click(verifyOf(C_OPEN)!);
    expect(must(tid("shift-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(
      errorText("roleNotAllowedHere", "refused"),
      "the shared table knows this code",
    ).not.toBe("refused");
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length, "no view is read again").toBe(before);
    expect(q(tid("shift-notice"))?.textContent ?? "").not.toBe(th("saved"));
  });

  it("a retry after a network error resends the same Idempotency-Key and a byte-equal body", async () => {
    stub();
    answers[`/api/handovers/${H_MINE}/accept`] = [
      "network",
      { status: 200, body: { id: H_MINE, status: "ACCEPTED" } },
    ];
    const { refresh } = await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN }),
    );
    await vi.waitFor(() =>
      expect(
        q(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`),
      ).not.toBeNull(),
    );
    await click(must(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`));
    expect(refresh).not.toHaveBeenCalled();
    await click(must(`[data-handover-id="${H_MINE}"] ${tid("shift-accept")}`));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBeTruthy();
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("two different counts verified one after the other use two different keys", async () => {
    stub();
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(verifyOf(C_CLOSE)).not.toBeNull());
    await click(verifyOf(C_CLOSE)!);
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).not.toBe(sent[0]!.key);
  });
});

// ── the real page ───────────────────────────────────────────────────────────────────────────

describe("the real /staff page mounts the shift card through main.tsx and cards.ts", () => {
  it('main.tsx, with the ./*/card.ts glob, renders data-card="shift"', async () => {
    const app = document.createElement("div");
    app.id = "app";
    document.body.appendChild(app);
    stub({
      staff_me: me(["ShiftOperate"]),
      staff_floor: floor({ myClockIn: CLOCKED_IN, myShift: null }),
      me: {
        kind: "user",
        id: ME,
        role: "any",
        branch: BRANCH,
        name: "Ploy",
        username: "ploy",
        branchName: "XC-B",
        branches: [{ id: BRANCH, code: "XC-B", name: "XC-B" }],
      },
    });
    vi.resetModules();
    await import("../main");
    await vi.waitFor(() =>
      expect(app.querySelector('[data-card="shift"]')).not.toBeNull(),
    );
    expect(
      app.querySelector(`[data-card="shift"] ${tid("shift-open-count")}`),
    ).not.toBeNull();
  });
});

// ── the folder's static rules ───────────────────────────────────────────────────────────────

describe("SD §8.1 money stays a string in web/src/screens/staff/**", () => {
  it("no Number-constructor call in any staff source (tests included)", () => {
    const pattern = new RegExp("\\b" + "Num" + "ber\\(");
    const hits: string[] = [];
    for (const [file, src] of Object.entries(staffSources)) {
      src.split("\n").forEach((line, i) => {
        if (pattern.test(line)) hits.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(
      Object.keys(staffSources),
      "the scan reaches shift/ (its own files key as ./)",
    ).toContain("./card.ts");
    expect(Object.keys(staffSources)).toContain("../cards.ts");
    expect(hits).toEqual([]);
  });

  it("no shift/ source writes through /api/staff/rpc (S5)", () => {
    for (const [file, src] of Object.entries(shiftSources)) {
      expect(src, file).not.toContain("/api/staff/rpc");
    }
  });
});
