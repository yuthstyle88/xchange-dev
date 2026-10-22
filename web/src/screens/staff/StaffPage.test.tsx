// S-3 #575: the staff page shell — frame, card registry, sign-in redirect, refresh, report link, hours line, texts.
// Design: docs/design/xchange-v3-wave3-staff.md SD:§4 (root, cards.ts), SD:S1, S2, S6, S7, S8, S12, SD:§6.1 B1–B2,
// SD:§8.1 row S-3. Written before the implementation by the ticket author (claude/709ec749, on the FE lane
// controller's order). Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion,
// not the compile):
// - `cards.ts`:
//     export interface StaffCard { id: string; order: number; visible(ctx: StaffCtx): boolean;
//                                  Component: (p: { ctx: StaffCtx }) => JSX.Element }
//     export type StaffCtx = { me: StaffMe; floor: StaffFloor | undefined; refresh: () => Promise<void> }
//     export function collectCards(mods: Record<string, { card: StaffCard }>): StaffCard[]   // sorted by order, then id
//     export const cards = collectCards(import.meta.glob('./*/card.ts', { eager: true }))
//   `StaffMe` / `StaffFloor` are S-1 #573's generated answers: `StaffMeView` (`web/src/shared/api/generated/
//   staff_me.ts`) and `StaffFloorView` (`staff_floor.ts`). The fixtures below are typed with them, so a fixture that
//   drifts from the wire fails the typecheck (updated at release, after #573 merged as 08c94225).
// - `StaffPage.tsx`: export function StaffPage(props: { api: ApiClient; cards: readonly StaffCard[];
//                                                        navigate: (url: string) => void }): JSX.Element
//   On mount it reads the views `staff_me` and `staff_floor` (`ApiClient.view`); it renders each visible card inside
//   an element with `data-card="<id>"`, in registry order; the hours line has `data-testid="staff-hours"`; the
//   `โหลดข้อมูลล่าสุด` button has `data-testid="staff-refresh"`; the report link is `a[href="/staff/report"]`.
// - Signed out (`staff_me` answers 401): `navigate("/login?next=%2Fstaff")` — OR-3's sign-in screen, then back to
//   /staff (SD:S1). No card is rendered.
// - `i18n.ts`: `th` and `en` tables with the same keys (namespace `staff.*`).
// - The hours line shows the branch's hours as HH:MM in Asia/Bangkok, the zone of every xchange branch today
//   (S12); with no `hours` it shows the sentence without any time.
//
// Permissions are spelled as `staff_me.permissions` carries them: serde's default for the `Permission` enum, the
// variant name (`BranchDayOperate`, `FinanceReconcile`, …), as #573's test serialises them.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../shared/i18n";
import { ApiClient } from "../../shared/api/client";
import {
  makeStaffFloorHours,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffMeView,
} from "../../shared/testing/fixtures";

type CardsModule = {
  collectCards: (mods: Record<string, { card: unknown }>) => Array<{ id: string; order: number }>;
};
type PageModule = {
  StaffPage: (p: {
    api: ApiClient;
    cards: readonly unknown[];
    navigate: (url: string) => void;
  }) => JSX.Element;
};
type TextsModule = { th: Record<string, string>; en: Record<string, string> };

const cardsMods = import.meta.glob<CardsModule>("./cards.ts");
const pageMods = import.meta.glob<PageModule>("./StaffPage.tsx");
const textMods = import.meta.glob<TextsModule>("./i18n.ts");
const SOURCES = import.meta.glob<string>(
  ["./main.tsx", "./StaffPage.tsx", "./cards.ts", "./parts/*.ts", "./parts/*.tsx"],
  { query: "?raw", import: "default", eager: true },
);
const PAGE_HTML = import.meta.glob<string>("../../../pages/staff.html", {
  query: "?raw",
  import: "default",
  eager: true,
});

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/staff/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

// ── fixtures: the views as S-1 answers them (SD:§3.1) ──────────────────────────────────────

type Me = StaffMeView;

function me(permissions: string[]) {
  // `roles` is deliberately empty: the page must decide from `permissions` only (SD:S2), and this file must hold no
  // role name (the §4 grep criterion covers it too).
  return makeStaffMeView({
    userId: "00000000-0000-4000-8000-0000000000a1",
    name: "Ploy",
    roles: [],
    branchId: "MAIN",
    branchName: "XC-B",
    permissions,
  });
}

const FLOOR_BASE = makeStaffFloorView({
  phase: "OPEN",
  hours: null,
  myClockIn: null,
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
});
const FLOOR_WITH_HOURS = makeStaffFloorView({
  ...FLOOR_BASE,
  hours: makeStaffFloorHours({ open: "2026-10-11T03:00:00Z", close: "2026-10-11T13:00:00Z" }), // 10:00–20:00 Bangkok
});
// S12: a branch with no hours set answers `hours: null` (#573's generated type).
const FLOOR_NO_HOURS = makeStaffFloorView({ ...FLOOR_BASE, phase: "READY" });

type Route = { status: number; body: unknown };
let routes: Record<string, () => Route>;
let fetchSpy: ReturnType<typeof vi.fn>;

function viewsOf(meAnswer: Route, floor: unknown = FLOOR_WITH_HOURS) {
  routes = {
    "/api/v3/views/staff_me": () => meAnswer,
    "/api/v3/views/staff_floor": () => ({
      status: 200,
      body: { version: 1, changed: true, data: floor },
    }),
  };
}

function ok(data: unknown): Route {
  return { status: 200, body: { version: 1, changed: true, data } };
}

const SIGNED_OUT: Route = {
  status: 401,
  body: { error: { code: "signIn", message: "กรุณาเข้าสู่ระบบ" } },
};

function viewCalls(name: string): number {
  return fetchSpy.mock.calls.filter((c) => String(c[0]).split("?")[0] === `/api/v3/views/${name}`)
    .length;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input).split("?")[0];
    const route = routes[path];
    if (!route) throw new Error(`unexpected request ${String(input)}`);
    const { status, body } = route();
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  setLang("th");
});

type Card = {
  id: string;
  order: number;
  visible: (ctx: { me: Me; floor: unknown; refresh: () => Promise<void> }) => boolean;
  Component: (p: { ctx: { me: Me; floor: unknown; refresh: () => Promise<void> } }) => JSX.Element;
};

function card(id: string, order: number, visible: Card["visible"] = () => true): Card {
  return {
    id,
    order,
    visible,
    Component: () => <p data-testid={`body-${id}`}>{id}</p>,
  };
}

async function mount(cards: Card[] = []) {
  const { StaffPage } = await load(pageMods, "./StaffPage.tsx");
  const navigate = vi.fn();
  const api = new ApiClient(fetchSpy as unknown as typeof fetch);
  dispose = render(() => <StaffPage api={api} cards={cards} navigate={navigate} />, host);
  return navigate;
}

// ── B1 / S1: signed out → OR-3's sign-in screen, then back to /staff ───────────────────────

describe("B1 signed out", () => {
  it("R1 a 401 from staff_me sends the visitor to the sign-in screen with next=/staff, and renders no card", async () => {
    viewsOf(SIGNED_OUT);
    const navigate = await mount([card("floor", 10)]);
    await vi.waitFor(() => expect(navigate).toHaveBeenCalledWith("/login?next=%2Fstaff"));
    expect(host.querySelector("[data-card]")).toBeNull();
  });

  it("R2 a signed-in visitor is not redirected", async () => {
    viewsOf(ok(me([])));
    const navigate = await mount();
    await vi.waitFor(() => expect(viewCalls("staff_me")).toBe(1));
    await vi.waitFor(() => expect(host.textContent ?? "").not.toBe(""));
    expect(navigate).not.toHaveBeenCalled();
  });

  it("R3 the page has no login card of its own: no password field", async () => {
    viewsOf(SIGNED_OUT);
    await mount();
    await vi.waitFor(() => expect(viewCalls("staff_me")).toBe(1));
    expect(host.querySelector('input[type="password"]')).toBeNull();
  });
});

// ── the card registry (SD:§4) ───────────────────────────────────────────────────────────────

describe("cards.ts: one card per folder, sorted by order", () => {
  it("K1 collectCards sorts by order, then id, whatever order the glob gives", async () => {
    const { collectCards } = await load(cardsMods, "./cards.ts");
    // `order` disagrees with the alphabetical order of the ids, so a sort by id alone fails; the tie (20) is broken
    // by id.
    const got = collectCards({
      "./floor/card.ts": { card: card("floor", 30) },
      "./shift/card.ts": { card: card("shift", 20) },
      "./transfers/card.ts": { card: card("transfers", 10) },
      "./rates/card.ts": { card: card("rates", 20) },
    });
    expect(got.map((c) => c.id)).toEqual(["transfers", "rates", "shift", "floor"]);
  });

  it("K2 an empty glob gives no cards (every card ticket is still to come)", async () => {
    const { collectCards } = await load(cardsMods, "./cards.ts");
    expect(collectCards({})).toEqual([]);
  });

  it("K3 cards.ts globs ./*/card.ts", () => {
    const src = SOURCES["./cards.ts"];
    expect(src, "cards.ts exists").toBeDefined();
    expect(src).toContain("./*/card.ts");
  });

  it("K4 a card whose visible(ctx) is false is not rendered; the visible ones render in registry order", async () => {
    viewsOf(ok(me(["AttendanceClock"])));
    const seen: unknown[] = [];
    await mount([
      card("floor", 10, (ctx) => {
        seen.push(ctx);
        return true;
      }),
      card("hidden", 20, () => false),
      card("rates", 30),
    ]);
    await vi.waitFor(() => expect(host.querySelector('[data-card="rates"]')).not.toBeNull());
    expect(host.querySelector('[data-card="hidden"]')).toBeNull();
    expect(host.querySelector('[data-testid="body-hidden"]')).toBeNull();
    const order = [...host.querySelectorAll("[data-card]")].map((e) => e.getAttribute("data-card"));
    expect(order).toEqual(["floor", "rates"]);
  });

  it("K5 visible() is given ctx.me from staff_me and ctx.floor from staff_floor", async () => {
    const who = me(["AttendanceClock"]);
    viewsOf(ok(who), FLOOR_NO_HOURS);
    let got: { me: Me; floor: unknown } | undefined;
    await mount([
      card("floor", 10, (ctx) => {
        got = ctx;
        return true;
      }),
    ]);
    await vi.waitFor(() => expect(got?.floor).toEqual(FLOOR_NO_HOURS));
    expect(got?.me).toEqual(who);
  });
});

// ── S6: no polling; reads on mount, on refresh, and through ctx.refresh ─────────────────────

describe("S6 reads", () => {
  it("D1 mount reads staff_me and staff_floor once each", async () => {
    viewsOf(ok(me([])));
    await mount();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(1));
    expect(viewCalls("staff_me")).toBe(1);
  });

  it("D2 โหลดข้อมูลล่าสุด reads both views again", async () => {
    viewsOf(ok(me([])));
    await mount();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(1));
    const button = host.querySelector<HTMLButtonElement>('[data-testid="staff-refresh"]');
    expect(button, "the refresh button").not.toBeNull();
    button!.click();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(2));
    expect(viewCalls("staff_me")).toBe(2);
  });

  it("D3 a card's ctx.refresh reads both views again (after every write)", async () => {
    viewsOf(ok(me([])));
    let refresh: (() => Promise<void>) | undefined;
    await mount([
      card("floor", 10, (ctx) => {
        refresh = ctx.refresh;
        return true;
      }),
    ]);
    await vi.waitFor(() => expect(refresh).toBeDefined());
    await refresh!();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(2));
    expect(viewCalls("staff_me")).toBe(2);
  });

  it("D4 no polling: ten minutes later nothing more was read", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    viewsOf(ok(me([])));
    await mount();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(1));
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(viewCalls("staff_floor")).toBe(1);
    expect(viewCalls("staff_me")).toBe(1);
  });

  it("D5 no setInterval anywhere in the shell's sources", () => {
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/setInterval\s*\(/);
    }
  });
});

// ── B2: the report link ─────────────────────────────────────────────────────────────────────

describe("B2 report link", () => {
  const cases: [string[], boolean][] = [
    [["ShiftCloseCertify"], true],
    [["FinanceReconcile"], true],
    [["OverviewRead"], true],
    [["BranchDayOperate"], false],
    [["AttendanceClock", "ShiftOperate", "CashManage", "CompatRead"], false],
    [[], false],
  ];
  for (const [permissions, shown] of cases) {
    it(`L ${JSON.stringify(permissions)} → report link ${shown ? "shown" : "hidden"}`, async () => {
      viewsOf(ok(me(permissions)));
      await mount();
      await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(1));
      await vi.waitFor(() => expect(host.textContent ?? "").not.toBe(""));
      expect(host.querySelector('a[href="/staff/report"]') !== null).toBe(shown);
    });
  }
});

// ── S12: the hours line ─────────────────────────────────────────────────────────────────────

describe("S12 hours line", () => {
  it("H1 the branch's hours are shown as HH:MM in Bangkok, whatever the machine's own zone", async () => {
    // The runners and dev machines are +07 themselves, so a format in the machine's local zone would pass by
    // accident; run this test in UTC so only a Bangkok format shows 10:00–20:00 (review by claude/14cc24bd).
    // `process` is reached the way AdminShell.test.tsx does it: the web tsconfig has no node types.
    const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } })
      .process.env;
    const saved = env.TZ;
    env.TZ = "UTC";
    try {
      viewsOf(ok(me(["AttendanceClock"])), FLOOR_WITH_HOURS);
      await mount();
      await vi.waitFor(() => {
        const line = host.querySelector('[data-testid="staff-hours"]');
        expect(line?.textContent ?? "").toContain("10:00");
        expect(line?.textContent ?? "").toContain("20:00");
      });
    } finally {
      if (saved === undefined) delete env.TZ;
      else env.TZ = saved;
    }
  });

  it("H2 a branch with no hours shows the sentence without any time, never an invented one", async () => {
    viewsOf(ok(me(["AttendanceClock"])), FLOOR_NO_HOURS);
    await mount();
    await vi.waitFor(() => expect(viewCalls("staff_floor")).toBe(1));
    await vi.waitFor(() =>
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent ?? "").not.toBe(""),
    );
    expect(host.querySelector('[data-testid="staff-hours"]')!.textContent).not.toMatch(
      /\d{1,2}:\d{2}/,
    );
  });
});

// ── S8: every text through t(), TH and EN ───────────────────────────────────────────────────

describe("S8 texts", () => {
  it("T1 i18n.ts has th and en tables with the same keys, none empty", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(th).length).toBeGreaterThan(0);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of [...Object.entries(th), ...Object.entries(en)]) {
      expect(v.trim(), k).not.toBe("");
    }
  });

  it("T2 the refresh button is โหลดข้อมูลล่าสุด in Thai, and its English text in English", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.values(th)).toContain("โหลดข้อมูลล่าสุด");
    viewsOf(ok(me([])));
    await mount();
    const button = () => host.querySelector('[data-testid="staff-refresh"]');
    await vi.waitFor(() => expect(button()?.textContent).toBe("โหลดข้อมูลล่าสุด"));
    setLang("en");
    const enRefresh = Object.entries(th).find(([, v]) => v === "โหลดข้อมูลล่าสุด")![0];
    await vi.waitFor(() => expect(button()?.textContent).toBe(en[enRefresh]));
  });

  it("T3 the shell's sources hold no Thai text: every text comes from i18n.ts", () => {
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, `${file} has no Thai letters`).not.toMatch(/[฀-๿]/);
    }
  });
});

// ── the page file ───────────────────────────────────────────────────────────────────────────

describe("web/pages/staff.html", () => {
  it("P1 is the /staff page and mounts screens/staff/main.tsx", () => {
    const html = Object.values(PAGE_HTML)[0];
    expect(html, "web/pages/staff.html exists").toBeDefined();
    expect(html).toMatch(/xc-path[^>]*\/staff\b/);
    expect(html).toContain("/src/screens/staff/main.tsx");
  });
});
