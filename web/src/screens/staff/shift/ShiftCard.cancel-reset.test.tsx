// #2660: Staff Shift card Cancel resets typed count and hand-over quantities and receiver
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { DenominationRow } from "../../../shared/api/generated/denominations";
import type { HandoverReceiverRow } from "../../../shared/api/generated/handover_receivers";
import type { MyShift } from "../../../shared/api/generated/my_shift";
import type { StaffCard, StaffCtx } from "../cards";
import { warmModules } from "../../../shared/testing/warm-modules";

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/shift/card.ts exists").toBeDefined();
  return (await l!()).card;
}

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
const MY_SHIFT_VIEW: MyShift = {
  id: MY_SHIFT.id,
  status: "OPEN",
  openedAt: MY_SHIFT.openedAt,
  balances: [],
};

const DENOMS: DenominationRow[] = [
  { id: 12, currency: "THB", faceValue: "100.0000", groupCode: "NOTE" },
  { id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" },
  { id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" },
];

const RECEIVERS: HandoverReceiverRow[] = [
  { userId: THIRD, name: "Aom" },
  { userId: ME, name: "Ploy" },
  { userId: OTHER, name: "Somchai" },
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
    cash_counts: [],
    handovers_pending: [],
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
      return json(200, { id: path.split("/")[3] ?? "ok", status: "ACCEPTED" });
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

async function clickCancel(form: HTMLFormElement) {
  const cancel = form.querySelector<HTMLButtonElement>(".xc-shift-cancel-btn");
  expect(cancel, "cancel button inside the form").not.toBeNull();
  cancel!.click();
  await settle();
}

async function saveCount(form: HTMLFormElement) {
  const save = form.querySelector<HTMLButtonElement>(tid("shift-count-save"));
  expect(save, `${tid("shift-count-save")} inside the form`).not.toBeNull();
  save!.click();
  await settle();
}

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
  host = document.createElement("div");
  document.body.appendChild(host);
  setLang("th");
});

afterEach(() => {
  unmount();
  host.remove();
  vi.restoreAllMocks();
});

describe("#2660 ShiftCard cancel resets count and handover forms", () => {
  it("C1 open count → type 7 on one denomination → Cancel → open count: that input is empty", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    let form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "7");
    expect(qtyInput(form, 11).value).toBe("7");

    await clickCancel(form);
    expect(q(tid("shift-count-form"))).toBeNull();

    form = await openCountForm("shift-open-count");
    expect(qtyInput(form, 11).value).toBe("");
  });

  it("C2 close count (with my OPEN shift) → type → Cancel → close count: empty", async () => {
    stub({ my_shift: MY_SHIFT_VIEW });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    let form = await openCountForm("shift-close-count");
    typeInto(qtyInput(form, 12), "4");
    expect(qtyInput(form, 12).value).toBe("4");

    await clickCancel(form);
    expect(q(tid("shift-count-form"))).toBeNull();

    form = await openCountForm("shift-close-count");
    expect(qtyInput(form, 12).value).toBe("");
  });

  it("H1 hand-over (with my OPEN shift) → type 3 on one denomination → Cancel → hand-over: that input is empty; no receiver selected", async () => {
    stub({ my_shift: MY_SHIFT_VIEW, handover_receivers: { rows: RECEIVERS } });
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: MY_SHIFT }),
    );
    let form = await openHandover();
    await vi.waitFor(() => expect(q(tid("shift-receiver"))).not.toBeNull());

    const receiverSelect = must<HTMLSelectElement>(tid("shift-receiver"));
    typeInto(receiverSelect as unknown as HTMLInputElement, OTHER);
    expect(receiverSelect.value).toBe(OTHER);

    typeInto(qtyInput(form, 11), "3");
    expect(qtyInput(form, 11).value).toBe("3");

    await clickCancel(form);
    expect(q(tid("shift-handover-form"))).toBeNull();

    form = await openHandover();
    expect(qtyInput(form, 11).value).toBe("");
    expect(must<HTMLSelectElement>(tid("shift-receiver")).value).toBe("");
  });

  it("C3 open count → type → Save (200) → open count: empty (keep, :192)", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    let form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "7");
    await saveCount(form);
    await vi.waitFor(() => expect(q(tid("shift-count-form"))).toBeNull());

    form = await openCountForm("shift-open-count");
    expect(qtyInput(form, 11).value).toBe("");
  });

  it("C4 open count → type → Cancel → open count → type 5 → Save: the body's lines carry only the 5", async () => {
    stub();
    await mount(
      me(["ShiftOperate"]),
      floor({ myClockIn: CLOCKED_IN, myShift: null }),
    );
    let form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 11), "7");
    typeInto(qtyInput(form, 12), "9");
    await clickCancel(form);

    form = await openCountForm("shift-open-count");
    typeInto(qtyInput(form, 21), "5");
    await saveCount(form);

    const post = sent.find((s) => s.url === "/api/cash-counts");
    expect(post, "POST /api/cash-counts sent").toBeDefined();
    const body = bodyOf(post!);
    expect(body["lines"]).toEqual([
      { currency: "USD", denominationId: 21, qty: "5" },
    ]);
  });
});
