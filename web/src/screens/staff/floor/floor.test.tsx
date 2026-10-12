// S-4 #577: the staff floor card (phase, clock in/out, start service, cut-off, the cash target, safe and target figures).
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order, the owner having ordered it on 2026-10-11). Do not edit, delete or weaken; add tests in a
// file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md (SD)
//   §4      floor/: phase, clock in/out (clock), start service and cut-off (dayStartService, dayCutoff;
//           BranchDayOperate), target form (dayTarget; CashTargetSet), safe and target figures. Texts staff.floor.*.
//   §6.1    B3 shown with can(AttendanceClock); B4 start when phase = READY, cut-off when phase = OPEN, both with
//           can(BranchDayOperate); B5 the form with can(CashTargetSet), the figures when staff_floor answers them.
//   §6.2    C1 clock {direction: IN | OUT}; C2 dayStartService {equipmentReady, ratesVerified, rateVersion} after a
//           ConfirmDialog with v2's sentence; C3 dayCutoff {}; C4 dayTarget {amounts: {CUR: str}}. After success:
//           notice "บันทึกแล้ว" and staff_floor re-read; on refusal: t('error.'+code), nothing re-read, the form keeps
//           its values. Every write carries an Idempotency-Key kept until success (parts/command.ts).
//   S2      no role literal; access is a Permission.   S5  nothing through /api/staff/rpc.
//   S8      every text through t(), TH and EN; v2's Thai in th.   §8.1  no Number-constructor call in staff/**.
// v2's page (SD's `S`, 108-Plaza/xchange-v2-source src/booking/app/staff/page.tsx @ 4d0bdbb, line 22): the
// start-service confirm sentence and the phase labels pinned below are copied from it.
//
// The contract this test assumes ([required] unless marked):
//   floor/card.ts     export const card: StaffCard   (../cards.ts: { id, order, visible(ctx), Component })
//                     id = 'floor' (the shell wraps it in data-card="floor"); visible reads only can(ctx.me, …).
//   floor/i18n.ts     export const th = {…}; export const en = {…}: the same keys, no zh table. Keys this test reads:
//                     'phase.READY' 'phase.OPEN' 'phase.CUT_OFF' 'phase.COUNTING' 'phase.CLOSED' clockIn clockOut
//                     startService startConfirm cutoff targetSave saved.
//   DOM hooks         [data-testid=floor-phase]       the phase, as t('phase.'+phase)
//                     [data-testid=floor-clock]       one button: t('clockIn') with no floor.myClockIn, else t('clockOut')
//                     [data-testid=floor-start]       start service (B4); opens parts/ConfirmDialog
//                     [data-testid=floor-cutoff]      cut-off (B4)
//                     [data-testid=floor-target-form] a <form> (B5) with one <input data-currency="CUR"> per currency it
//                                                     lists: THB first, then sorted and deduplicated every currency of
//                                                     staff_published_rates, floor.target.amounts and floor.safe (FE ruling,
//                                                     #577 issuecomment-6107759125); prefilled from floor.target.amounts;
//                                                     an empty input is not sent; and a submit button
//                                                     [data-testid=floor-target-save]
//                     [data-testid=floor-target] / [data-testid=floor-safe]  the figures, each amount exactly as the
//                                                     wire string (no reformatting), absent when staff_floor gives null
//                     [data-testid=floor-notice] / [data-testid=floor-error] the after-write lines
//   Commands          CA-8b's routes (docs/design/blueprint/cash-shift-transfer.md §4.4, #1274):
//                     POST /api/attendance (clock) · /api/branch-day/start · /api/branch-day/cutoff · /api/branch-day/target
//                     through parts/command.ts (header idempotency-key). A `branch` member in a body, if sent, must be
//                     the actor's own branch; it is the only member allowed beyond the design's.
//   Types             the card imports CA-8b's generated web/src/shared/api/generated/branch_day.ts (SD §6.2: "a renamed
//                     field is a compile error"). It is reached here through import.meta.glob, so this file goes red,
//                     never fails to compile, until #1274 merges.
//   rateVersion       the publication.publishedAt of GET /api/v3/views/staff_published_rates, the branch's current
//                     rate publication (SD C2; #1274 §7-8: any other value is refused with ServiceChecksIncomplete).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { PublishedRatesData } from "../../../shared/api/generated/published_rates";
import type { StaffCard, StaffCtx } from "../cards";
import { th as staffTh } from "../i18n";

type Texts = {
  th: Record<string, string>;
  en: Record<string, string>;
} & Record<string, unknown>;

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");
const textMods = import.meta.glob<Texts>("./i18n.ts", { eager: true });
const generated = import.meta.glob<string>(
  "../../../shared/api/generated/branch_day.ts",
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
);
const floorSources = import.meta.glob<string>(
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
  expect(l, "web/src/screens/staff/floor/card.ts exists").toBeDefined();
  return (await l!()).card;
}

function texts(): Texts {
  const m = textMods["./i18n.ts"];
  expect(m, "web/src/screens/staff/floor/i18n.ts exists").toBeDefined();
  return m!;
}

const KEYS = [
  "phase.READY",
  "phase.OPEN",
  "phase.CUT_OFF",
  "phase.COUNTING",
  "phase.CLOSED",
  "clockIn",
  "clockOut",
  "startService",
  "startConfirm",
  "cutoff",
  "targetSave",
  "saved",
] as const;

function th(key: (typeof KEYS)[number]): string {
  const v = texts().th[key];
  expect(v, `floor th['${key}']`).toBeTypeOf("string");
  return v!;
}

// ── fixtures ────────────────────────────────────────────────────────────────────────────────

const BRANCH = "MAIN";
const PUBLISHED_AT = "2026-10-11T02:30:00Z";

function me(permissions: string[]): StaffMeView {
  return {
    userId: "00000000-0000-4000-8000-0000000000f1",
    name: "Ploy",
    roles: [],
    branchId: BRANCH,
    branchName: "XC-B",
    permissions,
  };
}

const ALL = ["AttendanceClock", "BranchDayOperate", "CashTargetSet"];

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

const TARGET = {
  amounts: { USD: "1000.00", EUR: "500.50" },
  at: "2026-10-11T01:00:00Z",
};
const SAFE = { USD: "12500.00", JPY: "300000.00" };

const RATES: PublishedRatesData = {
  publication: {
    id: "11111111-2222-4333-8444-555555555555",
    version: 7,
    publishedAt: PUBLISHED_AT,
  },
  sourceDate: "2026-10-10",
  stale: false,
  lines: [
    {
      currency: "USD",
      group: "ALL",
      unitSize: "1",
      buyRate: "32.4500",
      sellRate: "32.9000",
      lineId: "l-usd",
    },
  ],
};

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};
type Answer = { status: number; body: unknown } | "network";

let sent: Sent[];
let answers: Record<string, Answer[]>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stub(extraViews: Record<string, unknown> = {}) {
  const views: Record<string, unknown> = {
    staff_published_rates: RATES,
    ...extraViews,
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
        if (name in views)
          return json(200, { version: 1, changed: true, data: views[name] });
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
      return json(200, {});
    }),
  );
}

function ctxOf(m: StaffMeView, f: StaffFloorView | undefined) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: f, refresh };
  return { ctx, refresh };
}

async function mount(m: StaffMeView, f: StaffFloorView | undefined) {
  const card = await loadCard();
  const { ctx, refresh } = ctxOf(m, f);
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
}

async function settle() {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
}

function q<T extends Element = HTMLElement>(id: string): T | null {
  return host.querySelector<T>(`[data-testid="${id}"]`);
}

function must<T extends Element = HTMLElement>(id: string): T {
  const el = q<T>(id);
  expect(el, `[data-testid="${id}"]`).not.toBeNull();
  return el!;
}

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

/** The body a command sent, as an object, without a `branch` member that names the actor's own branch. */
function bodyOf(s: Sent): Record<string, unknown> {
  expect(s.body, `${s.url} has a JSON body`).toBeTypeOf("string");
  const b = JSON.parse(s.body!) as Record<string, unknown>;
  if ("branch" in b) {
    expect(b["branch"], "a branch member names the actor\x27s own branch").toBe(
      BRANCH,
    );
    delete b["branch"];
  }
  return b;
}

function confirmButton(): HTMLButtonElement {
  const dialog = host.querySelector("dialog");
  expect(dialog, "the start-service ConfirmDialog is open").not.toBeNull();
  const b = [...dialog!.querySelectorAll("button")].find(
    (x) => x.textContent === staffTh.confirm,
  );
  expect(b, "the dialog\x27s confirm button").toBeDefined();
  return b as HTMLButtonElement;
}

function cancelButton(): HTMLButtonElement {
  const dialog = host.querySelector("dialog");
  expect(dialog).not.toBeNull();
  const b = [...dialog!.querySelectorAll("button")].find(
    (x) => x.textContent === staffTh.cancel,
  );
  expect(b, "the dialog\x27s cancel button").toBeDefined();
  return b as HTMLButtonElement;
}

function typeInto(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

async function submit(form: HTMLFormElement) {
  const save = form.querySelector<HTMLButtonElement>(
    '[data-testid="floor-target-save"]',
  );
  expect(
    save,
    '[data-testid="floor-target-save"] inside the form',
  ).not.toBeNull();
  save!.click();
  await settle();
}

beforeEach(() => {
  sent = [];
  answers = {};
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

// ── the card record and its texts ───────────────────────────────────────────────────────────

describe("S-4 the card record (SD:§4)", () => {
  it('exports card { id: "floor", order, visible, Component }', async () => {
    const card = await loadCard();
    expect(card.id).toBe("floor");
    expect(Number.isFinite(card.order)).toBe(true);
    expect(card.visible).toBeTypeOf("function");
    expect(card.Component).toBeTypeOf("function");
  });

  it("floor/i18n.ts has th and en with the same keys (every key this test reads), and no zh table (S8)", () => {
    const t = texts();
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const k of KEYS) {
      expect(t.th[k], `th['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}'] is English, not the Thai copied`).not.toBe(
        t.th[k],
      );
    }
    expect("zh" in t, "staff screens are TH/EN only").toBe(false);
  });

  it("th keeps v2's Thai where SD names it: the start-service sentence, the phase labels, 'บันทึกแล้ว'", () => {
    expect(th("startConfirm")).toBe(
      "ตรวจอุปกรณ์ ระบบ และเรทบนจอตรงกับชุดอนุมัติล่าสุดแล้ว?",
    );
    expect(th("saved")).toBe("บันทึกแล้ว");
    expect(th("phase.READY")).toBe("เตรียมกะแล้ว · รอยืนยันเปิดบริการ");
    expect(th("phase.OPEN")).toBe("เปิดรับรายการ");
    expect(th("phase.CUT_OFF")).toBe("ตัดรายการแล้ว / รอนับ");
    expect(th("phase.COUNTING")).toBe("กำลังตรวจและส่งมอบ");
    expect(th("phase.CLOSED")).toBe("ปิดกะ");
    expect(th("clockIn")).toBe("ลงเวลาเข้า");
  });

  it("the card's code imports CA-8b's generated branch_day types (SD §6.2), and that file exists", () => {
    expect(
      Object.keys(generated),
      "web/src/shared/api/generated/branch_day.ts (CA-8b #1274) exists",
    ).toHaveLength(1);
    const code = Object.entries(floorSources);
    expect(code.length, "floor/ has code besides its tests").toBeGreaterThan(0);
    expect(
      code.some(([, src]) =>
        /from\s+['"](\.\.\/){3}shared\/api\/generated\/branch_day['"]/.test(
          src,
        ),
      ),
      "a floor/ source imports ../../../shared/api/generated/branch_day",
    ).toBe(true);
  });
});

// ── B3–B5: who sees what ────────────────────────────────────────────────────────────────────

describe("B3 the card is shown with can(AttendanceClock), and only that", () => {
  it("visible exactly when the actor holds AttendanceClock", async () => {
    const card = await loadCard();
    const v = (perms: string[], f?: StaffFloorView) =>
      card.visible(ctxOf(me(perms), f).ctx);
    expect(v(["AttendanceClock"])).toBe(true);
    expect(v(["AttendanceClock"], floor())).toBe(true);
    expect(v(ALL, floor({ phase: "CLOSED" }))).toBe(true);
    expect(v([])).toBe(false);
    expect(
      v(
        ["BranchDayOperate", "CashTargetSet", "CashManage", "CompatRead"],
        floor(),
      ),
    ).toBe(false);
  });

  it("shows the phase as t(phase.<PHASE>) for every phase, in TH and in EN", async () => {
    for (const phase of ["READY", "OPEN", "CUT_OFF", "COUNTING", "CLOSED"]) {
      stub();
      await mount(me(["AttendanceClock"]), floor({ phase }));
      expect(must("floor-phase").textContent).toContain(
        texts().th[`phase.${phase}`]!,
      );
      dispose?.();
      host.innerHTML = "";
      setLang("en");
      await mount(me(["AttendanceClock"]), floor({ phase }));
      expect(must("floor-phase").textContent).toContain(
        texts().en[`phase.${phase}`]!,
      );
      dispose?.();
      dispose = undefined;
      host.innerHTML = "";
      setLang("th");
    }
  });

  it("the clock button says clockIn before a clock-in and clockOut after one", async () => {
    stub();
    await mount(me(["AttendanceClock"]), floor());
    expect(must("floor-clock").textContent).toBe(th("clockIn"));
    dispose?.();
    host.innerHTML = "";
    await mount(
      me(["AttendanceClock"]),
      floor({ myClockIn: "2026-10-11T01:00:00Z" }),
    );
    expect(must("floor-clock").textContent).toBe(th("clockOut"));
  });
});

describe("B4 start service at READY, cut-off at OPEN, each with can(BranchDayOperate)", () => {
  it("READY with BranchDayOperate: start shown, cut-off not", async () => {
    stub();
    await mount(me(ALL), floor({ phase: "READY" }));
    expect(must("floor-start").textContent).toBe(th("startService"));
    expect(q("floor-cutoff")).toBeNull();
  });

  it("OPEN with BranchDayOperate: cut-off shown, start not", async () => {
    stub();
    await mount(me(ALL), floor({ phase: "OPEN" }));
    expect(must("floor-cutoff").textContent).toBe(th("cutoff"));
    expect(q("floor-start")).toBeNull();
  });

  it("without BranchDayOperate neither is shown, whatever the phase", async () => {
    for (const phase of ["READY", "OPEN"]) {
      stub();
      await mount(me(["AttendanceClock", "CashTargetSet"]), floor({ phase }));
      expect(q("floor-start"), phase).toBeNull();
      expect(q("floor-cutoff"), phase).toBeNull();
      dispose?.();
      dispose = undefined;
      host.innerHTML = "";
    }
  });

  it("at CUT_OFF, COUNTING and CLOSED neither is shown, even with BranchDayOperate", async () => {
    for (const phase of ["CUT_OFF", "COUNTING", "CLOSED"]) {
      stub();
      await mount(me(ALL), floor({ phase }));
      expect(q("floor-start"), phase).toBeNull();
      expect(q("floor-cutoff"), phase).toBeNull();
      dispose?.();
      dispose = undefined;
      host.innerHTML = "";
    }
  });
});

describe("B5 the target form with can(CashTargetSet); the figures when staff_floor answers them", () => {
  it("the form is shown with CashTargetSet, prefilled from floor.target, and not without it", async () => {
    stub();
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: TARGET }),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    expect(form.tagName).toBe("FORM");
    const usd = form.querySelector<HTMLInputElement>(
      'input[data-currency="USD"]',
    );
    expect(usd, "an input for USD").not.toBeNull();
    expect(usd!.value).toBe("1000.00");
    dispose?.();
    host.innerHTML = "";
    await mount(
      me(["AttendanceClock", "BranchDayOperate"]),
      floor({ target: TARGET }),
    );
    expect(q("floor-target-form")).toBeNull();
  });

  it("the target and safe figures show each amount exactly as the wire string", async () => {
    stub();
    await mount(me(["AttendanceClock"]), floor({ target: TARGET, safe: SAFE }));
    const target = must("floor-target").textContent ?? "";
    expect(target).toContain("USD");
    expect(target).toContain("1000.00");
    expect(target).toContain("500.50");
    const safe = must("floor-safe").textContent ?? "";
    expect(safe).toContain("12500.00");
    expect(safe).toContain("300000.00");
  });

  it("no figures when staff_floor answers target and safe as null", async () => {
    stub();
    await mount(me(ALL), floor({ target: null, safe: null }));
    expect(q("floor-target")).toBeNull();
    expect(q("floor-safe")).toBeNull();
  });
});

// ── B5 the target form's currencies (FE lane controller's ruling, #577 issuecomment-6107759125) ──────
// THB first; then, sorted and without duplicates, every currency that is a line of staff_published_rates, a key of
// floor.target.amounts or a key of floor.safe; prefilled from target.amounts where it has a value. No hard-coded list.

function currenciesOf(form: HTMLFormElement): string[] {
  return [
    ...form.querySelectorAll<HTMLInputElement>("input[data-currency]"),
  ].map((i) => i.getAttribute("data-currency") ?? "");
}

function ratesWith(...currencies: string[]): PublishedRatesData {
  return {
    ...RATES,
    lines: currencies.map((currency) => ({
      currency,
      group: "ALL",
      unitSize: "1",
      buyRate: "1.0000",
      sellRate: "1.1000",
      lineId: `l-${currency}`,
    })),
  };
}

describe("B5 the target form lists THB, then every published, target or safe currency (#577 ruling)", () => {
  it("an empty target with USD and EUR published lists THB, EUR, USD, all empty", async () => {
    stub({ staff_published_rates: ratesWith("USD", "EUR") });
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: null, safe: null }),
    );
    await vi.waitFor(() =>
      expect(currenciesOf(must<HTMLFormElement>("floor-target-form"))).toEqual([
        "THB",
        "EUR",
        "USD",
      ]),
    );
    for (const i of must<HTMLFormElement>(
      "floor-target-form",
    ).querySelectorAll<HTMLInputElement>("input[data-currency]")) {
      expect(i.value, `${i.getAttribute("data-currency")} starts empty`).toBe(
        "",
      );
    }
  });

  it("a currency only in safe is still listed; published, target and safe merge sorted with no duplicates", async () => {
    stub({ staff_published_rates: ratesWith("USD", "EUR", "THB") });
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({
        target: { amounts: { USD: "1000.00", GBP: "20.00" }, at: TARGET.at },
        safe: { JPY: "300000.00", USD: "1.00" },
      }),
    );
    await vi.waitFor(() =>
      expect(currenciesOf(must<HTMLFormElement>("floor-target-form"))).toEqual([
        "THB",
        "EUR",
        "GBP",
        "JPY",
        "USD",
      ]),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    expect(
      form.querySelector<HTMLInputElement>('input[data-currency="USD"]')!.value,
      "prefilled from target",
    ).toBe("1000.00");
    expect(
      form.querySelector<HTMLInputElement>('input[data-currency="GBP"]')!.value,
      "prefilled from target",
    ).toBe("20.00");
    expect(
      form.querySelector<HTMLInputElement>('input[data-currency="JPY"]')!.value,
      "safe does not prefill",
    ).toBe("");
  });

  it("a currency left empty is not sent (proposed by the test writer: the server reads every amount as a number)", async () => {
    stub({ staff_published_rates: ratesWith("USD", "EUR") });
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: null, safe: null }),
    );
    await vi.waitFor(() =>
      expect(
        currenciesOf(must<HTMLFormElement>("floor-target-form")),
      ).toHaveLength(3),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    typeInto(
      form.querySelector<HTMLInputElement>('input[data-currency="THB"]')!,
      "200000.00",
    );
    await submit(form);
    expect(bodyOf(sent[0]!)).toEqual({ amounts: { THB: "200000.00" } });
  });

  it("no floor/ source hard-codes a currency list (THB alone may be named)", () => {
    const codes = [
      "USD",
      "EUR",
      "JPY",
      "GBP",
      "CNY",
      "HKD",
      "SGD",
      "AUD",
      "KRW",
      "MYR",
      "TWD",
      "CHF",
      "CAD",
      "NZD",
      "INR",
    ];
    const named = new RegExp(`['"\`](${codes.join("|")})['"\`]`);
    const list = /\[\s*(['"`][A-Z]{3}['"`]\s*,\s*)+['"`][A-Z]{3}['"`]/;
    const code = Object.entries(floorSources);
    expect(code.length, "floor/ has code besides its tests").toBeGreaterThan(0);
    for (const [file, src] of code) {
      src.split("\n").forEach((line, i) => {
        expect(
          named.test(line),
          `${file}:${i + 1} names a currency other than THB: ${line.trim()}`,
        ).toBe(false);
        expect(
          list.test(line),
          `${file}:${i + 1} is a list of currency codes: ${line.trim()}`,
        ).toBe(false);
      });
    }
  });
});

// ── C1–C4: what each control sends ──────────────────────────────────────────────────────────

describe("C1–C4 each control sends exactly its command and body", () => {
  it('C1 clock in sends POST /api/attendance {direction: "IN"}', async () => {
    stub();
    await mount(me(["AttendanceClock"]), floor());
    await click(must("floor-clock"));
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/attendance",
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ direction: "IN" });
    expect(sent[0]!.key, "an Idempotency-Key").toBeTruthy();
  });

  it('C1 clock out sends POST /api/attendance {direction: "OUT"}', async () => {
    stub();
    await mount(
      me(["AttendanceClock"]),
      floor({ myClockIn: "2026-10-11T01:00:00Z" }),
    );
    await click(must("floor-clock"));
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/attendance",
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ direction: "OUT" });
  });

  it("C2 start service asks first with v2's sentence; cancel sends nothing", async () => {
    stub();
    await mount(me(ALL), floor({ phase: "READY" }));
    await click(must("floor-start"));
    expect(host.querySelector("dialog")?.textContent).toContain(
      th("startConfirm"),
    );
    expect(sent, "nothing is sent before the confirmation").toEqual([]);
    await click(cancelButton());
    expect(sent).toEqual([]);
    expect(host.querySelector("dialog")).toBeNull();
  });

  it("C2 confirmed: POST /api/branch-day/start {equipmentReady: true, ratesVerified: true, rateVersion: <publishedAt>}", async () => {
    stub();
    await mount(me(ALL), floor({ phase: "READY" }));
    await click(must("floor-start"));
    await click(confirmButton());
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/branch-day/start",
    ]);
    expect(bodyOf(sent[0]!)).toEqual({
      equipmentReady: true,
      ratesVerified: true,
      rateVersion: PUBLISHED_AT,
    });
  });

  it("C3 cut-off sends POST /api/branch-day/cutoff {}", async () => {
    stub();
    await mount(me(ALL), floor({ phase: "OPEN" }));
    await click(must("floor-cutoff"));
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/branch-day/cutoff",
    ]);
    expect(bodyOf(sent[0]!)).toEqual({});
  });

  it("C4 target sends POST /api/branch-day/target {amounts: {CUR: str}} with the typed strings", async () => {
    stub();
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: TARGET }),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    typeInto(
      form.querySelector<HTMLInputElement>('input[data-currency="USD"]')!,
      "1500.00",
    );
    await submit(form);
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/branch-day/target",
    ]);
    const body = bodyOf(sent[0]!);
    expect(Object.keys(body)).toEqual(["amounts"]);
    const amounts = body["amounts"] as Record<string, unknown>;
    expect(amounts["USD"], "the typed value, as typed").toBe("1500.00");
    expect(amounts["EUR"], "an untouched currency keeps its value").toBe(
      "500.50",
    );
    for (const [cur, v] of Object.entries(amounts))
      expect(v, `${cur} is a string`).toBeTypeOf("string");
  });
});

// ── after a write ───────────────────────────────────────────────────────────────────────────

describe("SD §6.2 after a write: success, refusal, retry", () => {
  it("success: the notice is t(saved) and staff_floor is re-read once (ctx.refresh)", async () => {
    stub();
    const { refresh } = await mount(me(["AttendanceClock"]), floor());
    await click(must("floor-clock"));
    expect(must("floor-notice").textContent).toBe(th("saved"));
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("a refusal shows t('error.'+code), re-reads nothing, and the form keeps its values", async () => {
    stub();
    answers["/api/branch-day/target"] = [
      {
        status: 403,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: TARGET }),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    typeInto(
      form.querySelector<HTMLInputElement>('input[data-currency="USD"]')!,
      "1500.00",
    );
    await submit(form);
    expect(must("floor-error").textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(
      errorText("roleNotAllowedHere", "refused"),
      "the shared table knows this code",
    ).not.toBe("refused");
    expect(refresh).not.toHaveBeenCalled();
    expect(q("floor-notice")?.textContent ?? "").not.toBe(th("saved"));
    expect(
      must<HTMLFormElement>(
        "floor-target-form",
      ).querySelector<HTMLInputElement>('input[data-currency="USD"]')!.value,
    ).toBe("1500.00");
  });

  it("a retry after a network error resends the same Idempotency-Key and a byte-equal body (clock)", async () => {
    stub();
    answers["/api/attendance"] = ["network", { status: 200, body: {} }];
    const { refresh } = await mount(me(["AttendanceClock"]), floor());
    await click(must("floor-clock"));
    expect(refresh).not.toHaveBeenCalled();
    await click(must("floor-clock"));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBeTruthy();
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("a retry after a network error resends the same Idempotency-Key and a byte-equal body (target)", async () => {
    stub();
    answers["/api/branch-day/target"] = ["network", { status: 200, body: {} }];
    await mount(
      me(["AttendanceClock", "CashTargetSet"]),
      floor({ target: TARGET }),
    );
    const form = must<HTMLFormElement>("floor-target-form");
    typeInto(
      form.querySelector<HTMLInputElement>('input[data-currency="USD"]')!,
      "1500.00",
    );
    await submit(form);
    await submit(must<HTMLFormElement>("floor-target-form"));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
  });

  it("a new write after a success uses a new Idempotency-Key", async () => {
    stub();
    await mount(me(["AttendanceClock"]), floor());
    await click(must("floor-clock"));
    await click(must("floor-clock"));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).not.toBe(sent[0]!.key);
  });
});

// ── the real page ───────────────────────────────────────────────────────────────────────────

describe("the real /staff page mounts the floor card through main.tsx and cards.ts", () => {
  it('main.tsx, with the ./*/card.ts glob, renders data-card="floor" with its clock button', async () => {
    const app = document.createElement("div");
    app.id = "app";
    document.body.appendChild(app);
    stub({
      staff_me: me(["AttendanceClock"]),
      staff_floor: floor({ phase: "OPEN" }),
      me: {
        kind: "user",
        id: "00000000-0000-4000-8000-0000000000f1",
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
      expect(app.querySelector('[data-card="floor"]')).not.toBeNull(),
    );
    expect(
      app.querySelector('[data-card="floor"] [data-testid="floor-clock"]'),
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
      "the scan reaches floor/ (its own files key as ./)",
    ).toContain("./card.ts");
    expect(Object.keys(staffSources)).toContain("../cards.ts");
    expect(hits).toEqual([]);
  });

  it("no floor/ source writes through /api/staff/rpc (S5)", () => {
    for (const [file, src] of Object.entries(floorSources)) {
      expect(src, file).not.toContain("/api/staff/rpc");
    }
  });
});
