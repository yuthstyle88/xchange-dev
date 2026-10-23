// #539 DK-D: step 0, rate and amount (design docs/design/xchange-v3-wave3-desk.md §3.2 row 0, §3.6b row "Step 0 rate",
// §6 row RateStep, D-4, D-12; ticket #539 Part 2 §7–§9). Every test runs on both OWNER_DESK and STAFF_DESK (§10).
// Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion):
// - `steps/rate/step.tsx` exports `step: DeskStep` with `index: 0` and `component({ store })`.
// - `steps/rate/i18n.ts` exports `th` and `en` with the same keys; Thai is v2's copy verbatim (§3.6b row "Step 0 rate").
// - The form's controls carry v2's names: `select[name=type]`, `select[name=currency]`, `input[name=amount]`,
//   `input[name=referenceRate]`; the form submits with its `button[type=submit]`.
// - The step reads the store only (§5.3 names): `config`, `quote`, `quoteError`, `type`/`currency`/`amount`/
//   `referenceRate`, `entry`, and writes `setType`, `setCurrency`, `setAmount`, `setReferenceRate`, `go(1)`.
// - The rate shown is the quote's `finalRate` exactly as the server answered it, beside its `unitSize` and currency
//   (D-4, D-12): never typed, never reformatted.
//
// The store here is a stub built from signals with §5.3's names, so this file does not depend on `desk-store.ts`.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";

type StepModule = { step: { index: number; label: () => string; component: (p: { store: any }) => JSX.Element } };
type TextsModule = { th: Record<string, string>; en: Record<string, string> };
const stepMods = import.meta.glob<StepModule>("./step.tsx");
const textMods = import.meta.glob<TextsModule>("./i18n.ts");
const SOURCES = import.meta.glob<string>(["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/steps/rate/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const NO_SHIFT = "ยังไม่มีกะเปิด — เปิดกะที่หน้าพนักงาน › กะของฉัน › นับเปิดกะ แล้วให้ผู้มีสิทธิ์รับรองยืนยันยอด";
const NEED_SHIFT_AND_RATE = "ต้องมีกะเปิด (หน้าพนักงาน › กะของฉัน › นับเปิดกะ) และเรทของสกุลเงินนี้ก่อน จากนั้นโหลดหน้านี้ใหม่";
const HEADING = "เลือกเรทและจำนวนเงิน";
const NEXT = "ถัดไป · พาสปอร์ต / ลูกค้า →";

const PATHS = { OWNER_DESK: { kind: "owner" }, STAFF_DESK: { kind: "staff" } } as const;

function quote(finalRate: string, unitSize: string, currency: string) {
  return {
    id: "q-1",
    type: "BUY",
    channel: "DESK",
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: { id: "p", version: 1 },
    lines: [{ currency, shopSide: "BUY", quantity: "10000", unitSize, baseRate: finalRate, finalRate, thbAmount: "2195.00", feeThb: "0.00", settleThb: "2195.00", publishedRateLineId: "l" }],
    benefits: [],
    stale: false,
  };
}

function stubStore(path: unknown, init: { shiftOpen?: boolean; quote?: unknown; quoteError?: unknown; locked?: boolean } = {}) {
  const [config] = createSignal<any>({ shiftOpen: init.shiftOpen ?? true });
  const [q, setQ] = createSignal<any>(init.quote);
  const [qe, setQe] = createSignal<any>(init.quoteError);
  const [type, setType] = createSignal("BUY");
  const [currency, setCurrency] = createSignal("JPY");
  const [amount, setAmount] = createSignal("10000");
  const [referenceRate, setReferenceRate] = createSignal("");
  const [entry] = createSignal<any>(init.locked ? { kind: "kiosk", id: "b1", locked: true } : undefined);
  const [step] = createSignal(0);
  const store = {
    path,
    config,
    configError: () => undefined,
    quote: q,
    quoteError: qe,
    quoting: () => false,
    type,
    currency,
    amount,
    referenceRate,
    entry,
    step,
    setType: vi.fn((v: string) => setType(v)),
    setCurrency: vi.fn((v: string) => setCurrency(v)),
    setAmount: vi.fn((v: string) => setAmount(v)),
    setReferenceRate: vi.fn((v: string) => setReferenceRate(v)),
    requote: vi.fn(async () => q()),
    go: vi.fn(),
    _setQuote: setQ,
    _setQuoteError: setQe,
  };
  return store;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("step 0 must not call the network itself"))));
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang("th");
});

async function mount(store: ReturnType<typeof stubStore>) {
  const { step } = await load(stepMods, "./step.tsx");
  dispose = render(() => step.component({ store }), host);
  return step;
}

const control = (name: string) => host.querySelector<HTMLInputElement | HTMLSelectElement>(`[name="${name}"]`);
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]');

describe("steps/rate: the module", () => {
  it("R0 exports step 0 with a label from its own i18n table", async () => {
    const { step } = await load(stepMods, "./step.tsx");
    expect(step.index).toBe(0);
    expect(step.label().trim()).not.toBe("");
  });

  it("R0b i18n.ts has th and en with the same keys; th carries v2's step-0 copy verbatim", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    const values = Object.values(th);
    for (const s of [HEADING, NEXT, NO_SHIFT, NEED_SHIFT_AND_RATE]) expect(values, s).toContain(s);
  });

  it("R0c no money arithmetic and no fetch( in the step's sources (W7)", () => {
    expect(Object.keys(SOURCES)).toContain("./step.tsx");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});

for (const [which, path] of Object.entries(PATHS)) {
  describe(`steps/rate on ${which}`, () => {
    it("R1 shows the heading and the next button in Thai", async () => {
      await mount(stubStore(path, { quote: quote("21.95", "100", "JPY") }));
      expect(host.textContent).toContain(HEADING);
      expect(submit()?.textContent).toContain(NEXT);
    });

    it("R2 the rate is the quote's finalRate exactly as answered, beside its unit size and currency (D-4, D-12)", async () => {
      await mount(stubStore(path, { quote: quote("21.9500", "100", "JPY") }));
      const text = host.textContent ?? "";
      expect(text).toContain("21.9500");
      expect(text).toMatch(/100\s*JPY/);
    });

    it("R3 the rate is never an editable field", async () => {
      await mount(stubStore(path, { quote: quote("21.95", "100", "JPY") }));
      expect(control("rate"), "no rate input").toBeNull();
    });

    it("R4 typing an amount sets it exactly as typed (a string, no arithmetic)", async () => {
      const store = stubStore(path, { quote: quote("21.95", "100", "JPY") });
      await mount(store);
      const amount = control("amount") as HTMLInputElement;
      amount.value = "10000.50";
      amount.dispatchEvent(new Event("input", { bubbles: true }));
      amount.dispatchEvent(new Event("change", { bubbles: true }));
      expect(store.setAmount).toHaveBeenCalledWith("10000.50");
    });

    it("R5 changing type or currency writes them to the store (which re-quotes, D-5)", async () => {
      const store = stubStore(path, { quote: quote("21.95", "100", "JPY") });
      await mount(store);
      const type = control("type") as HTMLSelectElement;
      type.value = "SELL";
      type.dispatchEvent(new Event("change", { bubbles: true }));
      expect(store.setType).toHaveBeenCalledWith("SELL");
      const currency = control("currency") as HTMLSelectElement;
      const other = [...currency.options].map((o) => o.value).find((v) => v && v !== "JPY");
      if (other) {
        currency.value = other;
        currency.dispatchEvent(new Event("change", { bubbles: true }));
        expect(store.setCurrency).toHaveBeenCalledWith(other);
      } else {
        expect(currency.options.length, "a currency select").toBeGreaterThan(0);
      }
    });

    it("R6 with a quote and an open shift, submit goes to step 1", async () => {
      const store = stubStore(path, { quote: quote("21.95", "100", "JPY") });
      await mount(store);
      submit()!.click();
      await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(1));
    });

    it("R7 no open shift: the notice shows, and submit is refused with the shift-and-rate sentence", async () => {
      const store = stubStore(path, { shiftOpen: false, quote: quote("21.95", "100", "JPY") });
      await mount(store);
      expect(host.textContent).toContain(NO_SHIFT);
      submit()!.click();
      await vi.waitFor(() => expect(host.textContent).toContain(NEED_SHIFT_AND_RATE));
      expect(store.go).not.toHaveBeenCalled();
    });

    it("R8 a quote refused on currency: submit is refused with the same sentence", async () => {
      const store = stubStore(path, { quote: undefined, quoteError: { field: "currency", message: "rateUnavailable" } });
      await mount(store);
      submit()!.click();
      await vi.waitFor(() => expect(host.textContent).toContain(NEED_SHIFT_AND_RATE));
      expect(store.go).not.toHaveBeenCalled();
    });

    it("R9 a quote refused on amount: the server's message shows at the amount field", async () => {
      const store = stubStore(path, { quote: undefined, quoteError: { field: "quantity", message: "จำนวนต้องมากกว่าศูนย์" } });
      await mount(store);
      const amount = control("amount") as HTMLInputElement;
      const field = amount.closest("label, .field, [data-field]") ?? amount.parentElement!;
      expect(field.textContent).toContain("จำนวนต้องมากกว่าศูนย์");
      submit()!.click();
      await Promise.resolve();
      expect(store.go).not.toHaveBeenCalled();
    });

    it("R10 a locked entry (booking/kiosk) makes the four fields read-only (dh:185)", async () => {
      await mount(stubStore(path, { quote: quote("21.95", "100", "JPY"), locked: true }));
      for (const n of ["type", "currency", "amount", "referenceRate"]) {
        const c = control(n);
        expect(c, `${n} present`).not.toBeNull();
        const ro = (c as HTMLInputElement).readOnly || c!.disabled || c!.getAttribute("aria-readonly") === "true";
        expect(ro, `${n} is read-only when locked`).toBe(true);
      }
    });

    it("R11 the texts follow the language: English shows the en heading", async () => {
      const { en } = await load(textMods, "./i18n.ts");
      const { th } = await load(textMods, "./i18n.ts");
      const key = Object.entries(th).find(([, v]) => v === HEADING)![0];
      await mount(stubStore(path, { quote: quote("21.95", "100", "JPY") }));
      setLang("en");
      await vi.waitFor(() => expect(host.textContent).toContain(en[key]));
    });
  });
}
