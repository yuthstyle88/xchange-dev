import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
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
const T_IN_A = "e0000000-0000-4000-8000-000000000003";
const T_IN_B = "e0000000-0000-4000-8000-000000000004";
const T_RECEIVED_A = "e0000000-0000-4000-8000-000000000005";
const T_RECEIVED_B = "e0000000-0000-4000-8000-000000000006";

const TRANSFERS: CashTransferRow[] = [
  transfer(T_APPROVED_A, "APPROVED"),
  transfer(T_APPROVED_B, "APPROVED"),
  transfer(T_IN_A, "DISPATCHED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_IN_B, "DISPATCHED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_RECEIVED_A, "RECEIVED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_RECEIVED_B, "RECEIVED", "BRANCH", ELSEWHERE, BRANCH),
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
    cash_transfers: TRANSFERS,
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [{ id: ELSEWHERE, name: "Branch Elsewhere" }] },
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
      return json(200, { id: path.split("/")[3] ?? "created-1", status: "DRAFT" });
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

describe("#2725: Staff Transfers notice clearing on failure", () => {
  it("T1: A dispatch succeeds ('Saved'), then another dispatch fails -> No transfer-notice; transfer-error shows", async () => {
    stub();
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      { status: 200, body: { id: T_APPROVED_A, status: "DISPATCHED" } },
    ];
    answers[`/api/cash-transfers/${T_APPROVED_B}/dispatch`] = [
      { status: 500, body: { error: { code: "serverError", message: "Dispatch failed" } } },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    // First dispatch succeeds
    inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!.click();
    await settle();

    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();

    // Second dispatch fails
    inRow<HTMLButtonElement>(T_APPROVED_B, "transfer-dispatch")!.click();
    await settle();

    expect(q(tid("transfer-notice"))).toBeNull();
    expect(must(tid("transfer-error")).textContent).toBeTruthy();
  });

  it("T2: A receive succeeds, then a receive fails -> No transfer-notice; transfer-error shows", async () => {
    stub();
    answers[`/api/cash-transfers/${T_IN_A}/receive`] = [
      { status: 200, body: { id: T_IN_A, status: "RECEIVED" } },
    ];
    answers[`/api/cash-transfers/${T_IN_B}/receive`] = [
      { status: 500, body: { error: { code: "serverError", message: "Receive failed" } } },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_IN_A, "transfer-receive")).not.toBeNull(),
    );

    // First receive succeeds
    inRow<HTMLButtonElement>(T_IN_A, "transfer-receive")!.click();
    await settle();

    const formA = must<HTMLFormElement>(tid("transfer-receive-form"));
    const inputA = formA.querySelector<HTMLInputElement>(
      'input[data-denomination-id="21"]',
    )!;
    typeInto(inputA, "2");
    must<HTMLButtonElement>(tid("transfer-receive-save")).click();
    await settle();

    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();

    // Second receive fails
    inRow<HTMLButtonElement>(T_IN_B, "transfer-receive")!.click();
    await settle();

    const formB = must<HTMLFormElement>(tid("transfer-receive-form"));
    const inputB = formB.querySelector<HTMLInputElement>(
      'input[data-denomination-id="21"]',
    )!;
    typeInto(inputB, "2");
    must<HTMLButtonElement>(tid("transfer-receive-save")).click();
    await settle();

    expect(q(tid("transfer-notice"))).toBeNull();
    expect(must(tid("transfer-error")).textContent).toBeTruthy();
  });

  it("T3: A reconcile succeeds, then a reconcile fails -> No transfer-notice; transfer-error shows", async () => {
    stub();
    answers[`/api/cash-transfers/${T_RECEIVED_A}/reconcile`] = [
      { status: 200, body: { id: T_RECEIVED_A, status: "RECONCILED" } },
    ];
    answers[`/api/cash-transfers/${T_RECEIVED_B}/reconcile`] = [
      { status: 500, body: { error: { code: "serverError", message: "Reconcile failed" } } },
    ];

    await mount(me(["CashManage", "FinanceReconcile"]));
    await vi.waitFor(() =>
      expect(inRow(T_RECEIVED_A, "transfer-reconcile")).not.toBeNull(),
    );

    // First reconcile succeeds
    inRow<HTMLButtonElement>(T_RECEIVED_A, "transfer-reconcile")!.click();
    await settle();

    const dialogA = must("dialog");
    typeInto(dialogA.querySelector("input")!, "checked A");
    const confirmBtnA = [...dialogA.querySelectorAll("button")].find(
      (b) => b.textContent === staffTh.confirm,
    )!;
    confirmBtnA.click();
    await settle();

    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();

    // Second reconcile fails
    inRow<HTMLButtonElement>(T_RECEIVED_B, "transfer-reconcile")!.click();
    await settle();

    const dialogB = must("dialog");
    typeInto(dialogB.querySelector("input")!, "checked B");
    const confirmBtnB = [...dialogB.querySelectorAll("button")].find(
      (b) => b.textContent === staffTh.confirm,
    )!;
    confirmBtnB.click();
    await settle();

    expect(q(tid("transfer-notice"))).toBeNull();
    expect(must(tid("transfer-error")).textContent).toBeTruthy();
  });

  it("T4: A create succeeds, then a dispatch fails -> No transfer-notice; transfer-error shows", async () => {
    stub();
    answers["/api/cash-transfers"] = [
      { status: 200, body: { id: "new-transfer-1", status: "DRAFT" } },
    ];
    answers["/api/cash-transfers/new-transfer-1/submit"] = [
      { status: 200, body: { id: "new-transfer-1", status: "SUBMITTED" } },
    ];
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      { status: 500, body: { error: { code: "serverError", message: "Dispatch failed" } } },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() => expect(q(tid("transfer-new"))).not.toBeNull());

    // Create succeeds
    must(tid("transfer-new")).click();
    await settle();
    await vi.waitFor(() => expect(q(tid("transfer-form"))).not.toBeNull());
    const form = must<HTMLFormElement>(tid("transfer-form"));
    await vi.waitFor(() =>
      expect(form.querySelector('input[data-denomination-id="21"]')).not.toBeNull(),
    );

    typeInto(must<HTMLSelectElement>(tid("transfer-kind")), "EXTERNAL_OUT");
    await settle();
    typeInto(form.querySelector<HTMLInputElement>('input[data-denomination-id="21"]')!, "5");
    await settle();

    must<HTMLButtonElement>(tid("transfer-save")).click();
    await settle();

    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();

    // Now dispatch fails
    inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!.click();
    await settle();

    expect(q(tid("transfer-notice"))).toBeNull();
    expect(must(tid("transfer-error")).textContent).toBeTruthy();
  });

  it("T5: A write succeeds -> transfer-notice 'Saved', no error (kept)", async () => {
    stub();
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      { status: 200, body: { id: T_APPROVED_A, status: "DISPATCHED" } },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!.click();
    await settle();

    expect(must(tid("transfer-notice")).textContent).toBe(transfersTh.saved);
    expect(q(tid("transfer-error"))).toBeNull();
  });

  it("T6: A write fails with no earlier success -> transfer-error only (kept)", async () => {
    stub();
    answers[`/api/cash-transfers/${T_APPROVED_A}/dispatch`] = [
      { status: 500, body: { error: { code: "serverError", message: "Dispatch failed" } } },
    ];

    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED_A, "transfer-dispatch")).not.toBeNull(),
    );

    inRow<HTMLButtonElement>(T_APPROVED_A, "transfer-dispatch")!.click();
    await settle();

    expect(q(tid("transfer-notice"))).toBeNull();
    expect(must(tid("transfer-error")).textContent).toBeTruthy();
  });
});
