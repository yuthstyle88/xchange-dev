// EX-9 #1147: the admin export buttons follow `me.permissions` (`ReportsExport`), and an export the server refuses
// downloads nothing. Written before the implementation by the up-front test writer (claude/709ec749, on the BE lane
// controller claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// Ticket §3 and §9 (web rows): the three screens (overview, transactions, reports) show their four export buttons
// only when `me.permissions` contains 'ReportsExport'; clicking one walks admin_transactions with purpose=export;
// a `403 outOfScope` answer shows `errorText('outOfScope', …)` through the screen's existing error path and no file
// is downloaded. The server gate is the rule (bp:§02 "UI ซ่อนปุ่มอย่างเดียวไม่เพียงพอ"); this is its UI half.
//
// The screens are mounted as the admin shell mounts a menu (`<menu>.Component` with AdminMenuProps), each test with a
// fresh module graph so the shared session reads this test's `me` (#1030's shape: `permissions` among its keys).
// The buttons are found by their Thai labels: 'ส่งออกรายการ CSV' (overview, transactions, reports) and
// 'ดาวน์โหลด CSV' (the reports employee tab).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import type { AdminMenu } from "./menus";
import { warmModules } from "../../shared/testing/warm-modules";

const EXPORT = "ส่งออกรายการ CSV";
const EMPLOYEE_CSV = "ดาวน์โหลด CSV";
const BRANCH = "MAIN";

type Reply = { status: number; body: unknown };
type Screen = "overview" | "transactions" | "reports";

const BASE_PERMISSIONS = [
  "AttendanceClock",
  "CompatRead",
  "OverviewRead",
  "SignedIn",
];

function me(permissions: string[]) {
  return {
    id: "00000000-0000-4000-8000-0000000e9101",
    name: "Viewer",
    roles: [{ role: "FINANCE", branchId: BRANCH }],
    permissions: [...permissions].sort(),
    branch: BRANCH,
    branchName: "Silom #2",
    branches: [{ id: BRANCH, code: "XC-1", name: "Silom #2" }],
  };
}

function row(n: number) {
  return {
    id: `XC-20261006-${String(n).padStart(6, "0")}`,
    date: "2026-10-06",
    created: "2026-10-06T03:04:05.000Z",
    type: "BUY",
    currency: "USD",
    amount: "100.00",
    rate: "30.0000",
    unitSize: "1",
    principal: "3000.00",
    fee: "5.00",
    bankFee: "0.00",
    settlement: "2994.99",
    method: "Cash",
    customer: `Customer ${n}`,
    customerId: `CUS-${n}`,
    staff: "dao",
    status: "COMPLETED",
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    shiftId: null,
  };
}

const OVERVIEW = {
  period: { from: "2026-10-05", to: "2026-10-06" },
  totals: {
    volume: "3000.00",
    buyPrincipal: "3000.00",
    sellPrincipal: "0.00",
    buyCount: 1,
    sellCount: 0,
    buyShare: "100",
    sellShare: "0",
    received: "0.00",
    paid: "3000.00",
    net: "-3000.00",
    fees: "0.00",
    completedCount: 1,
    cancelledCount: 0,
    pendingCount: 0,
  },
  latest: [row(1)],
  status: { shiftOpen: true, pendingCount: 0, openedBy: "Somchai" },
};

const REPORT = {
  period: { from: "2026-10-05", to: "2026-10-06" },
  stats: {
    received: "763.00",
    paid: "3025.00",
    fees: "8.00",
    bankFees: "1.00",
  },
  byChannel: [],
  byCurrency: [],
  reference: {
    referenced: 0,
    unreferenced: 0,
    value: "0.00",
    spread: "0.00",
    percent: null,
  },
  byEmployee: [
    {
      staff: "dao",
      count: 1,
      completed: 1,
      pending: 0,
      cancelled: 0,
      received: "0.00",
      paid: "3000.00",
      net: "-3000.00",
      fees: "5.00",
      bankFees: "0.00",
      methods: [],
      currencies: [],
    },
  ],
  employeeTotals: {
    people: 1,
    completed: 1,
    received: "0.00",
    paid: "3000.00",
    net: "-3000.00",
  },
};

let reads: URL[];
let exportAnswer: Reply | undefined;
let permissions: string[];
let blobs: Blob[];
let root: HTMLElement | undefined;
let dispose: (() => void) | undefined;
const originalCreate = URL.createObjectURL;
const originalRevoke = URL.revokeObjectURL;

function ok(data: unknown): Reply {
  return { status: 200, body: { version: 1, changed: true, data } };
}

function route(url: URL): Reply {
  const view = url.pathname.replace("/api/v3/views/", "");
  switch (view) {
    case "me":
      return ok(me(permissions));
    case "kiosk_notifications":
      return { status: 200, body: { version: 1, changed: false } };
    case "admin_overview":
      return ok(OVERVIEW);
    case "admin_reports":
      return ok(REPORT);
    case "admin_branches":
      return ok({ branches: [{ id: BRANCH, name: "Silom #2" }] });
    case "admin_shift_state":
      return ok({ shiftOpen: true, deskUrl: "/desk?branch=MAIN" });
    case "admin_transactions":
      if (url.searchParams.get("purpose") === "export" && exportAnswer)
        return exportAnswer;
      return ok([row(1), row(2)]);
    default:
      return { status: 200, body: { version: 1, changed: false } };
  }
}

async function settle(): Promise<void> {
  for (let n = 0; n < 4; n++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(screen: Screen): Promise<HTMLElement> {
  vi.resetModules();
  const mod = (await import(`./${screen}/menu.ts`)) as { menu: AdminMenu };
  const { Toast } = await import("./parts/toast");
  const [branch] = createSignal(BRANCH);
  const [period] = createSignal({ from: "2026-10-05", to: "2026-10-06" });
  const [shown] = createSignal(true);
  const Component = mod.menu.Component;
  root = document.createElement("div");
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  await vi.waitFor(() =>
    expect(reads.some((u) => u.pathname === "/api/v3/views/me")).toBe(true),
  );
  await settle();
  return root;
}

function buttons(el: ParentNode, label: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>("button")].filter(
    (b) => b.textContent?.trim() === label,
  );
}

function exportReads(): URL[] {
  return reads.filter(
    (u) =>
      u.pathname === "/api/v3/views/admin_transactions" &&
      u.searchParams.has("purpose"),
  );
}

beforeAll(
  () =>
    warmModules(
      () => import("./overview/menu"),
      () => import("./transactions/menu"),
      () => import("./reports/menu"),
      () => import("./parts/toast"),
      () => import("../../shared/i18n"),
    ),
  60_000,
);

beforeEach(() => {
  reads = [];
  exportAnswer = undefined;
  permissions = [...BASE_PERMISSIONS];
  blobs = [];
  URL.createObjectURL = ((b: Blob) => {
    blobs.push(b);
    return `blob:test/${blobs.length}`;
  }) as typeof URL.createObjectURL;
  URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input), "http://xc.test");
      reads.push(url);
      const reply = route(url);
      return new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { "content-type": "application/json" },
      });
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  URL.createObjectURL = originalCreate;
  URL.revokeObjectURL = originalRevoke;
});

describe("EX-9 the export buttons follow me.permissions", () => {
  it("without ReportsExport, none of the four export buttons render on overview, transactions or reports", async () => {
    const overview = await mount("overview");
    expect(buttons(overview, EXPORT), "overview").toHaveLength(0);
    dispose?.();
    root?.remove();
    const transactions = await mount("transactions");
    expect(buttons(transactions, EXPORT), "transactions").toHaveLength(0);
    dispose?.();
    root?.remove();
    const reports = await mount("reports");
    expect(buttons(reports, EXPORT), "reports live CSV").toHaveLength(0);
    expect(buttons(reports, EMPLOYEE_CSV), "reports employee CSV").toHaveLength(
      0,
    );
    expect(exportReads(), "nothing is exported").toEqual([]);
  });

  it("with ReportsExport, all four render", async () => {
    permissions = [...BASE_PERMISSIONS, "ReportsExport"];
    const overview = await mount("overview");
    expect(buttons(overview, EXPORT), "overview").toHaveLength(1);
    dispose?.();
    root?.remove();
    const transactions = await mount("transactions");
    expect(buttons(transactions, EXPORT), "transactions").toHaveLength(1);
    dispose?.();
    root?.remove();
    const reports = await mount("reports");
    expect(buttons(reports, EXPORT), "reports live CSV").toHaveLength(1);
    expect(buttons(reports, EMPLOYEE_CSV), "reports employee CSV").toHaveLength(
      1,
    );
  });

  it("clicking each of the four walks admin_transactions with purpose=export and downloads a file", async () => {
    permissions = [...BASE_PERMISSIONS, "ReportsExport"];
    const cases: [Screen, string][] = [
      ["overview", EXPORT],
      ["transactions", EXPORT],
      ["reports", EXPORT],
      ["reports", EMPLOYEE_CSV],
    ];
    for (const [screen, label] of cases) {
      const el = await mount(screen);
      const before = exportReads().length;
      const blobsBefore = blobs.length;
      buttons(el, label)[0]!.click();
      await settle();
      const walk = exportReads().slice(before);
      expect(walk.length, `${screen} ${label}: an export walk`).toBeGreaterThan(
        0,
      );
      for (const u of walk)
        expect(u.searchParams.get("purpose"), `${screen} ${label}`).toBe(
          "export",
        );
      expect(blobs.length, `${screen} ${label}: one file`).toBe(
        blobsBefore + 1,
      );
      dispose?.();
      root?.remove();
    }
    const plain = reads.filter(
      (u) =>
        u.pathname === "/api/v3/views/admin_transactions" &&
        !u.searchParams.has("purpose"),
    );
    for (const u of plain) expect(u.searchParams.has("purpose")).toBe(false);
  });

  it("a 403 outOfScope answer to an export shows errorText(outOfScope) and downloads nothing", async () => {
    permissions = [...BASE_PERMISSIONS, "ReportsExport"];
    exportAnswer = {
      status: 403,
      body: {
        error: {
          code: "outOfScope",
          message: "สาขานี้อยู่นอกขอบเขตของบัญชีนี้",
        },
      },
    };
    for (const screen of ["overview", "transactions", "reports"] as Screen[]) {
      const el = await mount(screen);
      const { errorText } = await import("../../shared/i18n");
      const expected = errorText(
        "outOfScope",
        "สาขานี้อยู่นอกขอบเขตของบัญชีนี้",
      );
      buttons(el, EXPORT)[0]!.click();
      await settle();
      expect(
        el.textContent ?? "",
        `${screen}: the screen shows the refusal`,
      ).toContain(expected);
      expect(blobs, `${screen}: no file`).toEqual([]);
      dispose?.();
      root?.remove();
    }
  });
});
