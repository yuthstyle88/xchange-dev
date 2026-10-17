import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { CashTransferRow } from "../../../shared/api/generated/cash_transfers";
import type { DenominationRow } from "../../../shared/api/generated/denominations";
import type { StaffCard, StaffCtx } from "../cards";
import { th as staffTh } from "../i18n";
import { warmModules } from "../../../shared/testing/warm-modules";
import { th as transfersTh } from "./i18n";

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/transfers/card.ts exists").toBeDefined();
  return (await l!()).card;
}

const BRANCH = "MAIN";
const ELSEWHERE = "B09";
const ME = "00000000-0000-4000-8000-0000000000b1";

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
    myClockIn: "2026-10-11T01:00:00Z",
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

const DENOMS: DenominationRow[] = [
  { id: 12, currency: "THB", faceValue: "100.0000", groupCode: "NOTE" },
  { id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" },
  { id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" },
];

function transfer(
  id: string,
  status: string,
  kind = "BRANCH",
  from = BRANCH,
  to: string | null = ELSEWHERE,
): CashTransferRow {
  return {
    id,
    kind,
    fromBranchId: from,
    toBranchId: to,
    fromShiftId: null,
    toShiftId: null,
    status,
    createdBy: "00000000-0000-4000-8000-0000000000b2",
    note: "n",
    createdAt: "2026-10-11T02:00:00Z",
    lines: [],
  };
}

const T_APPROVED_A = "e0000000-0000-4000-8000-000000000002";
const T_APPROVED_B = "e0000000-0000-4000-8000-000000000008";
const T_IN = "e0000000-0000-4000-8000-000000000003";
const T_RECEIVED = "e0000000-0000-4000-8000-000000000005";

const TRANSFERS: CashTransferRow[] = [
  transfer(T_APPROVED_A, "APPROVED"),
  transfer(T_APPROVED_B, "APPROVED"),
  transfer(T_IN, "DISPATCHED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_RECEIVED, "RECEIVED", "BRANCH", ELSEWHERE, BRANCH),
];

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};
type Answer =
  | { status: number; body: unknown }
  | "network"
  | (() => Promise<Response>);

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

function deferredAnswer(status: number, body: unknown): {
  answer: () => Promise<Response>;
  resolve: () => void;
  rejectNetwork: () => void;
} {
  let resolveFn!: (res: Response) => void;
  let rejectFn!: (err: unknown) => void;
  const promise = new Promise<Response>((res, rej) => {
    resolveFn = res;
    rejectFn = rej;
  });
  return {
    answer: () => promise,
    resolve: () => resolveFn(json(status, body)),
    rejectNetwork: () => rejectFn(new TypeError("Failed to fetch")),
  };
}

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = {
    cash_transfers: TRANSFERS,
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [] },
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
      if (typeof next === "function") return next();
      if (next === "network") throw new TypeError("Failed to fetch");
      if (next) return json(next.status, next.body);
      return json(200, { id: path.split("/")[3], status: "DRAFT" });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView) {
  const card = await loadCard();
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: floor(), refresh };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
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

function inRow<T extends HTMLElement = HTMLElement>(
  id: string,
  control: string,
): T | null {
  return q<T>(`[data-transfer-id="${id}"] ${tid(control)}`);
}

function typeInto(
  input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

beforeAll(
  () =>
    warmModules(
      () => import("./card"),
      () => import("../main"),
    ),
  60_000,
);

beforeEach(() => {
  sent = [];
  reads = [];
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

describe("TransfersCard in-flight guard (#2275)", () => {
  it("Double click Dispatch: sends one POST, shows notice 'saved', and shows no error", async () => {
    stub();
    const first = deferredAnswer(200, { id: T_APPROVED_A, status: "DISPATCHED" });
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      first.answer,
      {
        status: 409,
        body: {
          error: { code: "idempotencyConflict", message: "request in progress" },
        },
      },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    const btn = inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!;
    btn.click();
    expect(btn.disabled, "dispatch button disabled while in flight").toBe(true);
    btn.click();
    await settle();

    expect(sent).toHaveLength(1);
    first.resolve();
    await settle();

    expect(sent).toHaveLength(1);
    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();
    expect(
      inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!.disabled,
      "dispatch button re-enabled after settle",
    ).toBe(false);
  });

  it("Double submit Receive: sends one POST and disables receive save while in flight", async () => {
    stub();
    const first = deferredAnswer(200, { id: T_IN, status: "RECEIVED" });
    answers[`/api/cash-transfers/${T_IN}/receive`] = [
      first.answer,
      {
        status: 409,
        body: {
          error: { code: "idempotencyConflict", message: "request in progress" },
        },
      },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_IN, "transfer-receive")).not.toBeNull(),
    );

    inRow<HTMLButtonElement>(T_IN, "transfer-receive")!.click();
    await settle();

    const form = must<HTMLFormElement>(tid("transfer-receive-form"));
    const input = form.querySelector<HTMLInputElement>(
      'input[data-denomination-id="21"]',
    )!;
    typeInto(input, "2");

    const saveBtn = must<HTMLButtonElement>(tid("transfer-receive-save"));
    saveBtn.click();
    expect(saveBtn.disabled, "receive save button disabled while in flight").toBe(
      true,
    );
    form.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );
    await settle();

    expect(sent).toHaveLength(1);
    first.resolve();
    await settle();

    expect(sent).toHaveLength(1);
    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();
  });

  it("Double confirm Reconcile: sends one POST while in flight", async () => {
    stub();
    const first = deferredAnswer(200, { id: T_RECEIVED, status: "RECONCILED" });
    answers[`/api/cash-transfers/${T_RECEIVED}/reconcile`] = [
      first.answer,
      {
        status: 409,
        body: {
          error: { code: "idempotencyConflict", message: "request in progress" },
        },
      },
    ];

    await mount(me(["CashManage", "FinanceReconcile"]));
    await vi.waitFor(() =>
      expect(inRow(T_RECEIVED, "transfer-reconcile")).not.toBeNull(),
    );

    const reconcileBtn = inRow<HTMLButtonElement>(
      T_RECEIVED,
      "transfer-reconcile",
    )!;
    reconcileBtn.click();
    await settle();

    const dialog = must("dialog");
    typeInto(dialog.querySelector("input")!, "checked");
    const confirmBtn = [...dialog.querySelectorAll("button")].find(
      (b) => b.textContent === staffTh.confirm,
    )!;

    confirmBtn.click();
    expect(
      reconcileBtn.disabled,
      "row reconcile button disabled while in flight",
    ).toBe(true);
    confirmBtn.click();
    await settle();

    expect(sent).toHaveLength(1);
    first.resolve();
    await settle();

    expect(sent).toHaveLength(1);
    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();
  });

  it("Dispatch rows A and B one after the other: both are sent while per-row busy state isolates each row", async () => {
    stub();
    const firstA = deferredAnswer(200, { id: T_APPROVED_A, status: "DISPATCHED" });
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [firstA.answer];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    const btnA = inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!;
    const btnB = inRow<HTMLButtonElement>(T_APPROVED_B, "transfer-dispatch")!;

    btnA.click();
    expect(btnA.disabled, "row A is busy").toBe(true);
    expect(btnB.disabled, "row B is not blocked by row A").toBe(false);
    btnA.click();

    firstA.resolve();
    await settle();

    inRow<HTMLButtonElement>(T_APPROVED_B, "transfer-dispatch")!.click();
    await settle();

    expect(sent.map((s) => s.url)).toEqual([
      `/api/cash-transfers/${T_APPROVED_A}/dispatch`,
      `/api/cash-transfers/${T_APPROVED_B}/dispatch`,
    ]);
  });

  it("A retried dispatch after a network error: clears busy flag in finally and resends the same key and body", async () => {
    stub();
    const firstAttempt = deferredAnswer(200, { id: T_APPROVED_A });
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      firstAttempt.answer,
      { status: 200, body: { id: T_APPROVED_A } },
    ];

    const { refresh } = await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    const btn = inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!;
    btn.click();
    expect(btn.disabled, "busy during first network attempt").toBe(true);
    btn.click();
    expect(sent).toHaveLength(1);

    firstAttempt.rejectNetwork();
    await settle();

    expect(btn.disabled, "busy flag cleared in finally after network error").toBe(
      false,
    );

    btn.click();
    await settle();

    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("A refused dispatch: shows error.<code> and clears busy flag after refusal", async () => {
    stub();
    const firstAttempt = deferredAnswer(409, {
      error: { code: "roleNotAllowedHere", message: "refused" },
    });
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      firstAttempt.answer,
    ];

    const { refresh } = await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    const before = reads.length;
    const btn = inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!;
    btn.click();
    expect(btn.disabled, "busy while dispatch request is in flight").toBe(true);
    btn.click();
    expect(sent).toHaveLength(1);

    firstAttempt.resolve();
    await settle();

    expect(must(tid("transfer-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(btn.disabled, "busy cleared after refusal").toBe(false);
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length).toBe(before);
  });
});
