// S-7 #580 (web): the staff attendance card (the attendance log, read only).
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order, the owner having ordered it on 2026-10-11). Do not edit, delete or weaken; add tests in a
// file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md (SD)
//   §4      attendance/: the attendance log (staff_attendance). Texts staff.attendance.*. Its own card (ticket §7).
//   §6.1    B10 can(AttendanceClock): own rows; all rows for BranchDayOperate, decided by the server (S-1 #573): the
//           card shows exactly the rows the view answers and filters nothing itself.
//   S4      paged, newest first, as the view answers.   S6 no polling.   S2 no role literal · S8 TH/EN.
// View: S-1's staff_attendance, rows {id, name, in, out: string | null} (generated/staff_attendance.ts).
//
// The contract this test assumes ([required] unless marked):
//   attendance/card.ts  export const card: StaffCard; id = 'attendance'; shown exactly with can(AttendanceClock).
//   attendance/i18n.ts  th / en, same keys, no zh. Key read here: stillIn (the text of a row with no out).
//   DOM hooks           [data-attendance-id="<id>"] one element per row, in the view's order, showing the row's name;
//                       a row whose out is null shows t(stillIn).
//   Reads               staff_attendance through ApiClient.view; the card sends no command.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { StaffAttendanceRow } from "../../../shared/api/generated/staff_attendance";
import type { StaffCard, StaffCtx } from "../cards";

type Texts = {
  th: Record<string, string>;
  en: Record<string, string>;
} & Record<string, unknown>;

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");
const textMods = import.meta.glob<Texts>("./i18n.ts", { eager: true });
const ownSources = import.meta.glob<string>(
  ["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"],
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
);

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/attendance/card.ts exists").toBeDefined();
  return (await l!()).card;
}

function texts(): Texts {
  const m = textMods["./i18n.ts"];
  expect(m, "web/src/screens/staff/attendance/i18n.ts exists").toBeDefined();
  return m!;
}

const BRANCH = "MAIN";
const ME = "00000000-0000-4000-8000-0000000000d1";

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

function floor(): StaffFloorView {
  return {
    phase: "OPEN",
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

/** As the view answers them: newest first. */
const ROWS: StaffAttendanceRow[] = [
  { id: "att-3", name: "Ploy", in: "2026-10-11T03:00:00Z", out: null },
  {
    id: "att-2",
    name: "Somchai",
    in: "2026-10-11T01:00:00Z",
    out: "2026-10-11T02:30:00Z",
  },
  {
    id: "att-1",
    name: "Malee",
    in: "2026-10-10T01:00:00Z",
    out: "2026-10-10T10:00:00Z",
  },
];

let methods: string[];
let reads: string[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = { staff_attendance: ROWS, ...views };
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
      methods.push(`${method} ${path}`);
      const name = path.startsWith("/api/v3/views/")
        ? path.slice("/api/v3/views/".length)
        : "";
      if (method === "GET") reads.push(name || path);
      if (method === "GET" && name in all) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: all[name] }),
          {
            status: 200,
            headers: { "content-type": "application/json" },
          },
        );
      }
      if (path === "/health/ready") {
        return new Response(
          JSON.stringify({
            version: "3.0.0",
            build: "abc1234",
            channel: "staging",
            builtAt: "2026-10-11T00:00:00Z",
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }
      return new Response(
        JSON.stringify({ error: { code: "notFound", message: "not found" } }),
        {
          status: 404,
          headers: { "content-type": "application/json" },
        },
      );
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView) {
  const card = await loadCard();
  const ctx: StaffCtx = {
    me: m,
    floor: floor(),
    refresh: vi.fn(async () => {}),
  };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
}

beforeEach(() => {
  methods = [];
  reads = [];
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

describe("S-7 the attendance card record (SD:§4)", () => {
  it('exports card { id: "attendance", order, visible, Component }', async () => {
    const card = await loadCard();
    expect(card.id).toBe("attendance");
    expect(Number.isFinite(card.order)).toBe(true);
    expect(card.visible).toBeTypeOf("function");
    expect(card.Component).toBeTypeOf("function");
  });

  it("attendance/i18n.ts has th and en with the same keys, and no zh table", () => {
    const t = texts();
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    expect(t.th["stillIn"]).toBeTypeOf("string");
    expect(t.en["stillIn"]).toBeTypeOf("string");
    expect(t.en["stillIn"]).not.toBe(t.th["stillIn"]);
    expect("zh" in t).toBe(false);
  });

  it("the card's code reads staff_attendance's generated type and writes nothing", () => {
    const code = Object.values(ownSources).join("\n");
    expect(
      Object.keys(ownSources).length,
      "attendance/ has code besides its tests",
    ).toBeGreaterThan(0);
    expect(code).toMatch(
      /from\s+['"](\.\.\/){3}shared\/api\/generated\/staff_attendance['"]/,
    );
    expect(code).not.toContain("/api/staff/rpc");
  });
});

describe("B10 the attendance log with can(AttendanceClock)", () => {
  it("visible exactly when the actor holds AttendanceClock", async () => {
    const card = await loadCard();
    const v = (perms: string[]) =>
      card.visible({ me: me(perms), floor: floor(), refresh: async () => {} });
    expect(v(["AttendanceClock"])).toBe(true);
    expect(v(["AttendanceClock", "BranchDayOperate"])).toBe(true);
    expect(v([])).toBe(false);
    expect(
      v(["BranchDayOperate", "CashManage", "ShiftOperate", "CompatRead"]),
    ).toBe(false);
  });

  it("shows exactly the rows the view answers, in its order, with each row's name: no filtering of its own", async () => {
    stub();
    await mount(me(["AttendanceClock"]));
    await vi.waitFor(() =>
      expect(host.querySelectorAll("[data-attendance-id]")).toHaveLength(3),
    );
    const order = [...host.querySelectorAll("[data-attendance-id]")].map((e) =>
      e.getAttribute("data-attendance-id"),
    );
    expect(order).toEqual(["att-3", "att-2", "att-1"]);
    expect(
      host.querySelector('[data-attendance-id="att-2"]')!.textContent,
    ).toContain("Somchai");
    expect(reads).toContain("staff_attendance");
  });

  it("a row with no out shows t(stillIn), in TH and in EN", async () => {
    stub();
    await mount(me(["AttendanceClock"]));
    await vi.waitFor(() =>
      expect(host.querySelector('[data-attendance-id="att-3"]')).not.toBeNull(),
    );
    expect(
      host.querySelector('[data-attendance-id="att-3"]')!.textContent,
    ).toContain(texts().th["stillIn"]!);
    expect(
      host.querySelector('[data-attendance-id="att-2"]')!.textContent,
    ).not.toContain(texts().th["stillIn"]!);
    setLang("en");
    await settle();
    expect(
      host.querySelector('[data-attendance-id="att-3"]')!.textContent,
    ).toContain(texts().en["stillIn"]!);
  });

  it("the card sends no command and reads staff_attendance once on mount (S6)", async () => {
    stub();
    await mount(me(["AttendanceClock"]));
    await vi.waitFor(() =>
      expect(host.querySelectorAll("[data-attendance-id]")).toHaveLength(3),
    );
    await settle();
    expect(
      methods.filter((m) => !m.startsWith("GET ")),
      "no write",
    ).toEqual([]);
    expect(
      reads.filter((r) => r === "staff_attendance").length,
      "read once on mount",
    ).toBe(1);
  });
});

describe("the real /staff page mounts the attendance card through main.tsx and cards.ts", () => {
  it('main.tsx, with the ./*/card.ts glob, renders data-card="attendance"', async () => {
    const app = document.createElement("div");
    app.id = "app";
    document.body.appendChild(app);
    stub({
      staff_me: me(["AttendanceClock"]),
      staff_floor: floor(),
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
      expect(app.querySelector('[data-card="attendance"]')).not.toBeNull(),
    );
    await vi.waitFor(() =>
      expect(
        app.querySelectorAll('[data-card="attendance"] [data-attendance-id]'),
      ).toHaveLength(3),
    );
  });
});
