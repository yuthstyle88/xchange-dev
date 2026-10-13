// S-7 #580 (web): the staff holds card (place a hold on a customer found by a typed key; the holds list; release).
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order, the owner having ordered it on 2026-10-11). Do not edit, delete or weaken; add tests in a
// file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md (SD)
//   §4      holds/: hold form (staff_customers search, holdCreate; CustomerHoldPlace), holds list (staff_holds), release
//           (holdRelease; VerificationReview). Texts staff.holds.*.
//   §6.1    B9 form can(CustomerHoldPlace); list can(CustomerHoldPlace) || can(VerificationReview); release
//           can(VerificationReview).
//   §6.2    C10 holdCreate {customerId, reason}; C11 holdRelease {id, note} (ReasonDialog "ผลตรวจและเหตุผลปลดพัก").
//           After success: notice "บันทึกแล้ว", staff_floor and the card's lists re-read; on refusal t('error.'+code),
//           nothing re-read, the form keeps its values.
//   S11     the customer is found by a typed key (name, customer id or passport number); nothing is listed until a
//           key is typed.   S2 no role literal · S5 nothing through /api/staff/rpc · S8 TH/EN · §8.1 no Number.
// Server half: S-2 staff_customers (this ticket's api/tests/v3_staff_customers_view_test.rs): GET
//   /api/v3/views/staff_customers?q=<key> → data [{id, displayName}]; S-1's staff_holds rows {id, customerId,
//   customerName, reason, status: OPEN | RELEASED, at} (generated/staff_holds.ts).
// Commands: CA-8b #1274 (cash-shift-transfer.md §4.4, routes [proposed] there): POST /api/customer-holds and
//   POST /api/customer-holds/{id}/release; their types in generated/branch_day.ts, reached here through
//   import.meta.glob until #1274 merges (then main is merged in, no rebase).
//
// The contract this test assumes ([required] unless marked):
//   holds/card.ts      export const card: StaffCard; id = 'holds'; shown with can(CustomerHoldPlace) or
//                      can(VerificationReview).
//   holds/i18n.ts      th / en, same keys, no zh. Keys read here: find place release releaseTitle saved; th releaseTitle
//                      = 'ผลตรวจและเหตุผลปลดพัก', th saved = 'บันทึกแล้ว'.
//   DOM hooks          [data-testid=holds-form]          the hold <form> (B9), holding <input data-testid=holds-key>,
//                                                        a [data-testid=holds-find] button, one
//                                                        [data-customer-id="<id>"] element per customer found (a click
//                                                        picks it), <input|textarea data-testid=holds-reason> and
//                                                        [data-testid=holds-place]
//                      [data-hold-id="<id>"]             one element per row of staff_holds, holding
//                                                        [data-testid=holds-release] on an OPEN hold when I may release
//                      [data-testid=holds-notice] / [data-testid=holds-error]
//   Reads              staff_holds through ApiClient.view; staff_customers with the typed key as `q`, only after a
//                      non-blank key is typed and holds-find pressed.
//   Bodies             a `branch` member, if sent, must be the actor's own branch; nothing else beyond the design's.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { StaffHoldRow } from "../../../shared/api/generated/staff_holds";
import type { StaffCard, StaffCtx } from "../cards";
import { th as staffTh } from "../i18n";

type Texts = {
  th: Record<string, string>;
  en: Record<string, string>;
} & Record<string, unknown>;

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");
const textMods = import.meta.glob<Texts>("./i18n.ts", { eager: true });
const generated = import.meta.glob<string>(
  [
    "../../../shared/api/generated/branch_day.ts",
    "../../../shared/api/generated/staff_customers.ts",
  ],
  { query: "?raw", import: "default", eager: true },
);
const ownSources = import.meta.glob<string>(
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
  expect(l, "web/src/screens/staff/holds/card.ts exists").toBeDefined();
  return (await l!()).card;
}

function texts(): Texts {
  const m = textMods["./i18n.ts"];
  expect(m, "web/src/screens/staff/holds/i18n.ts exists").toBeDefined();
  return m!;
}

const KEYS = ["find", "place", "release", "releaseTitle", "saved"] as const;

function th(key: (typeof KEYS)[number]): string {
  const v = texts().th[key];
  expect(v, `holds th['${key}']`).toBeTypeOf("string");
  return v!;
}

// ── fixtures ────────────────────────────────────────────────────────────────────────────────

const BRANCH = "MAIN";
const ME = "00000000-0000-4000-8000-0000000000c1";

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

const H_OPEN = "h-open-1";
const H_DONE = "h-released-1";
const HOLDS: StaffHoldRow[] = [
  {
    id: H_OPEN,
    customerId: "CX-00000001",
    customerName: "Somchai Jaidee",
    reason: "AML check",
    status: "OPEN",
    at: "2026-10-11T02:00:00Z",
  },
  {
    id: H_DONE,
    customerId: "CX-00000002",
    customerName: null,
    reason: "Old",
    status: "RELEASED",
    at: "2026-10-10T02:00:00Z",
  },
];

const FOUND = [
  { id: "CX-00000001", displayName: "Somchai Jaidee" },
  { id: "CX-00000007", displayName: "Somchai Jaiyen" },
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
    staff_holds: HOLDS,
    staff_customers: FOUND,
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
      const [path, query = ""] = url.split("?");
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "GET") {
        const name = path!.startsWith("/api/v3/views/")
          ? path!.slice("/api/v3/views/".length)
          : "";
        reads.push(query ? `${name || path}?${query}` : name || path!);
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
        url: path!,
        method,
        key: headers.get("idempotency-key"),
        body: init?.body as string | undefined,
      });
      const next = answers[path!]?.shift();
      if (next === "network") throw new TypeError("Failed to fetch");
      if (next) return json(next.status, next.body);
      return json(200, { id: "h-new" });
    }),
  );
}

function ctxOf(m: StaffMeView) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: floor(), refresh };
  return { ctx, refresh };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView) {
  const card = await loadCard();
  const { ctx, refresh } = ctxOf(m);
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

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

function typeInto(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function bodyOf(s: Sent): Record<string, unknown> {
  expect(s.body, `${s.url} has a JSON body`).toBeTypeOf("string");
  const b = JSON.parse(s.body!) as Record<string, unknown>;
  if ("branch" in b) {
    expect(b["branch"], "a branch member names the actor's own branch").toBe(
      BRANCH,
    );
    delete b["branch"];
  }
  return b;
}

function customerReads(): string[] {
  return reads.filter((r) => r.startsWith("staff_customers"));
}

/** Types `key`, finds, picks `customerId`, types `reason`. */
async function fillHold(key: string, customerId: string, reason: string) {
  typeInto(must<HTMLInputElement>(tid("holds-key")), key);
  await click(must(tid("holds-find")));
  await vi.waitFor(() =>
    expect(q(`[data-customer-id="${customerId}"]`)).not.toBeNull(),
  );
  await click(must(`[data-customer-id="${customerId}"]`));
  typeInto(must<HTMLInputElement>(tid("holds-reason")), reason);
  await settle();
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
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

// ── the card record and its texts ───────────────────────────────────────────────────────────

describe("S-7 the holds card record (SD:§4)", () => {
  it('exports card { id: "holds", order, visible, Component }', async () => {
    const card = await loadCard();
    expect(card.id).toBe("holds");
    expect(Number.isFinite(card.order)).toBe(true);
    expect(card.visible).toBeTypeOf("function");
    expect(card.Component).toBeTypeOf("function");
  });

  it("holds/i18n.ts has th and en with the same keys, no zh; the release dialog title is SD's Thai", () => {
    const t = texts();
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const k of KEYS) {
      expect(t.th[k], `th['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}'] is English`).not.toBe(t.th[k]);
    }
    expect(th("releaseTitle")).toBe("ผลตรวจและเหตุผลปลดพัก");
    expect(th("saved")).toBe("บันทึกแล้ว");
    expect("zh" in t, "staff screens are TH/EN only").toBe(false);
  });

  it("the generated files exist, and the card imports branch_day.ts (commands) and staff_customers.ts", () => {
    for (const f of ["branch_day", "staff_customers"]) {
      expect(
        Object.keys(generated).some((k) => k.endsWith(`/generated/${f}.ts`)),
        `web/src/shared/api/generated/${f}.ts exists`,
      ).toBe(true);
    }
    const code = Object.values(ownSources).join("\n");
    expect(
      Object.keys(ownSources).length,
      "holds/ has code besides its tests",
    ).toBeGreaterThan(0);
    for (const f of ["branch_day", "staff_customers", "staff_holds"]) {
      expect(
        new RegExp(
          `from\\s+['"](\\.\\./){3}shared/api/generated/${f}['"]`,
        ).test(code),
        `a holds/ source imports ../../../shared/api/generated/${f}`,
      ).toBe(true);
    }
  });
});

// ── B9 ──────────────────────────────────────────────────────────────────────────────────────

describe("B9 who sees what", () => {
  it("visible exactly with CustomerHoldPlace or VerificationReview", async () => {
    const card = await loadCard();
    const v = (perms: string[]) => card.visible(ctxOf(me(perms)).ctx);
    expect(v(["CustomerHoldPlace"])).toBe(true);
    expect(v(["VerificationReview"])).toBe(true);
    expect(v([])).toBe(false);
    expect(
      v(["AttendanceClock", "ShiftOperate", "CashManage", "CompatRead"]),
    ).toBe(false);
  });

  it("the form with CustomerHoldPlace; not with VerificationReview alone", async () => {
    stub();
    await mount(me(["CustomerHoldPlace"]));
    expect(must<HTMLFormElement>(tid("holds-form")).tagName).toBe("FORM");
    dispose?.();
    dispose = undefined;
    host.innerHTML = "";
    stub();
    await mount(me(["VerificationReview"]));
    expect(q(tid("holds-form"))).toBeNull();
  });

  it("the list shows every row of staff_holds, for either permission", async () => {
    for (const perms of [["CustomerHoldPlace"], ["VerificationReview"]]) {
      stub();
      await mount(me(perms));
      await vi.waitFor(() =>
        expect(q(`[data-hold-id="${H_OPEN}"]`)).not.toBeNull(),
      );
      expect(q(`[data-hold-id="${H_DONE}"]`), `${perms}`).not.toBeNull();
      expect(must(`[data-hold-id="${H_OPEN}"]`).textContent).toContain(
        "AML check",
      );
      dispose?.();
      dispose = undefined;
      host.innerHTML = "";
    }
  });

  it("release on an OPEN hold with VerificationReview only; never on a RELEASED one", async () => {
    stub();
    await mount(me(["VerificationReview"]));
    await vi.waitFor(() =>
      expect(
        q(`[data-hold-id="${H_OPEN}"] ${tid("holds-release")}`),
      ).not.toBeNull(),
    );
    expect(
      must(`[data-hold-id="${H_OPEN}"] ${tid("holds-release")}`).textContent,
    ).toContain(th("release"));
    expect(q(`[data-hold-id="${H_DONE}"] ${tid("holds-release")}`)).toBeNull();
    dispose?.();
    dispose = undefined;
    host.innerHTML = "";
    stub();
    await mount(me(["CustomerHoldPlace"]));
    await vi.waitFor(() =>
      expect(q(`[data-hold-id="${H_OPEN}"]`)).not.toBeNull(),
    );
    expect(
      host.querySelector(tid("holds-release")),
      "no release without VerificationReview",
    ).toBeNull();
  });
});

// ── S11: nothing is listed until a key is typed ─────────────────────────────────────────────

describe("S11 the customer is found by a typed key", () => {
  it("before a key is typed, nothing is listed and staff_customers is not read", async () => {
    stub();
    await mount(me(["CustomerHoldPlace"]));
    expect(host.querySelector("[data-customer-id]")).toBeNull();
    expect(customerReads()).toEqual([]);
    await click(must(tid("holds-find")));
    typeInto(must<HTMLInputElement>(tid("holds-key")), "   ");
    await click(must(tid("holds-find")));
    expect(
      host.querySelector("[data-customer-id]"),
      "a blank key lists nothing",
    ).toBeNull();
    expect(customerReads(), "a blank key reads nothing").toEqual([]);
  });

  it("a typed key reads staff_customers with that key as q and lists each answer by its displayName", async () => {
    stub();
    await mount(me(["CustomerHoldPlace"]));
    typeInto(must<HTMLInputElement>(tid("holds-key")), "Somchai J");
    await click(must(tid("holds-find")));
    await vi.waitFor(() =>
      expect(q(`[data-customer-id="CX-00000001"]`)).not.toBeNull(),
    );
    expect(customerReads().length).toBeGreaterThan(0);
    const params = new URLSearchParams(
      customerReads().at(-1)!.split("?")[1] ?? "",
    );
    expect(params.get("q")).toBe("Somchai J");
    expect(must(`[data-customer-id="CX-00000007"]`).textContent).toContain(
      "Somchai Jaiyen",
    );
  });
});

// ── C10 / C11 ───────────────────────────────────────────────────────────────────────────────

describe("C10 and C11 send exactly their command and body", () => {
  it("C10 place: POST /api/customer-holds {customerId, reason} for the picked customer", async () => {
    stub();
    await mount(me(["CustomerHoldPlace"]));
    await fillHold("Somchai", "CX-00000007", "Large cash");
    await click(must(tid("holds-place")));
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/customer-holds",
    ]);
    expect(bodyOf(sent[0]!)).toEqual({
      customerId: "CX-00000007",
      reason: "Large cash",
    });
    expect(sent[0]!.key).toBeTruthy();
  });

  it("C10 with no customer picked sends nothing", async () => {
    stub();
    await mount(me(["CustomerHoldPlace"]));
    typeInto(must<HTMLInputElement>(tid("holds-reason")), "Large cash");
    await click(must(tid("holds-place")));
    expect(sent).toEqual([]);
  });

  it("C11 release asks in ReasonDialog titled t(releaseTitle); confirm sends /{id}/release {id, note}; cancel nothing", async () => {
    stub();
    await mount(me(["VerificationReview"]));
    await vi.waitFor(() =>
      expect(
        q(`[data-hold-id="${H_OPEN}"] ${tid("holds-release")}`),
      ).not.toBeNull(),
    );
    await click(must(`[data-hold-id="${H_OPEN}"] ${tid("holds-release")}`));
    let dialog = must("dialog");
    expect(dialog.textContent).toContain(th("releaseTitle"));
    await click(
      [...dialog.querySelectorAll("button")].find(
        (b) => b.textContent === staffTh.cancel,
      )!,
    );
    expect(sent).toEqual([]);
    await click(must(`[data-hold-id="${H_OPEN}"] ${tid("holds-release")}`));
    dialog = must("dialog");
    typeInto(dialog.querySelector("input")!, "documents verified");
    await click(
      [...dialog.querySelectorAll("button")].find(
        (b) => b.textContent === staffTh.confirm,
      )!,
    );
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/customer-holds/${H_OPEN}/release`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({
      id: H_OPEN,
      note: "documents verified",
    });
  });
});

// ── after a write ───────────────────────────────────────────────────────────────────────────

describe("SD §6.2 after a write", () => {
  it("success: the notice is t(saved), staff_floor is re-read (ctx.refresh) and staff_holds is re-read", async () => {
    stub();
    const { refresh } = await mount(me(["CustomerHoldPlace"]));
    await fillHold("Somchai", "CX-00000001", "AML");
    const before = reads.filter((r) => r.startsWith("staff_holds")).length;
    await click(must(tid("holds-place")));
    await vi.waitFor(() =>
      expect(must(tid("holds-notice")).textContent).toBe(th("saved")),
    );
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(
      reads.filter((r) => r.startsWith("staff_holds")).length,
    ).toBeGreaterThan(before);
  });

  it("a refusal shows t('error.'+code), re-reads nothing and keeps the picked customer and the reason", async () => {
    stub();
    answers["/api/customer-holds"] = [
      {
        status: 403,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(me(["CustomerHoldPlace"]));
    await fillHold("Somchai", "CX-00000007", "Large cash");
    const before = reads.length;
    await click(must(tid("holds-place")));
    expect(must(tid("holds-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length).toBe(before);
    expect(must<HTMLInputElement>(tid("holds-reason")).value).toBe(
      "Large cash",
    );
    await click(must(tid("holds-place")));
    expect(bodyOf(sent[1]!), "the picked customer is kept").toEqual({
      customerId: "CX-00000007",
      reason: "Large cash",
    });
  });

  it("a retry after a network error resends the same key and a byte-equal body", async () => {
    stub();
    answers["/api/customer-holds"] = [
      "network",
      { status: 200, body: { id: "h-new" } },
    ];
    const { refresh } = await mount(me(["CustomerHoldPlace"]));
    await fillHold("Somchai", "CX-00000001", "AML");
    await click(must(tid("holds-place")));
    expect(refresh).not.toHaveBeenCalled();
    await click(must(tid("holds-place")));
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});

// ── the real page ───────────────────────────────────────────────────────────────────────────

describe("the real /staff page mounts the holds card through main.tsx and cards.ts", () => {
  it('main.tsx, with the ./*/card.ts glob, renders data-card="holds"', async () => {
    const app = document.createElement("div");
    app.id = "app";
    document.body.appendChild(app);
    stub({
      staff_me: me(["CustomerHoldPlace"]),
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
      expect(app.querySelector('[data-card="holds"]')).not.toBeNull(),
    );
    expect(
      app.querySelector(`[data-card="holds"] ${tid("holds-form")}`),
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
      "the scan reaches holds/ (its own files key as ./)",
    ).toContain("./card.ts");
    expect(hits).toEqual([]);
  });

  it("no holds/ source writes through /api/staff/rpc (S5)", () => {
    for (const [file, src] of Object.entries(ownSources)) {
      expect(src, file).not.toContain("/api/staff/rpc");
    }
  });
});
