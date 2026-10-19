// #543 Part 1, DK-G: step 3, payment (design docs/design/xchange-v3-wave3-desk.md §3.2 rows "3 Payment",
// "3 payments (10-06)", "3 cash count", §3.6 B2–B3, §3.6b row "Step 3 payment", D-4, D-7, §5.3 step-3 names,
// §5.4, §6 row PaymentStep). Every test runs on both OWNER_DESK and STAFF_DESK (§10). Written before the implementation
// by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's order). Do not edit, delete or weaken
// these tests; add your own elsewhere.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion):
// - `steps/payment/step.tsx` exports `step: DeskStep` with `index: 3` and `component({ store })`.
// - `steps/payment/i18n.ts` exports `th` and `en` with the same keys; Thai is v2's copy verbatim.
// - Controls, by v2's names (dh:117): `select[name=method]` with v2's six options in v2's order, `input[name=ref]`,
//   `input[name=account]`, `input[name=fee]`, `input[name=bankFee]`, `[name=notes]`, `input[type=file][name=slip]`,
//   `input[type=checkbox][name=paymentChecked]`; the form submits with its `button[type=submit]`; cancel is v2's
//   `ยกเลิกการจอง / แก้ไข`. An entry's identity checkbox is `input[type=checkbox][name=identityConfirmed]`.
// - The channel rule is v2's `desk-payments.js` (D-7), read from `store.config().paymentMethods` (generated
//   `DeskPaymentMethod`): an option is enabled when its row is `enabled` and its `buy`/`sell` flag is on for
//   `store.type()` (BUY → `buy`, SELL → `sell`); `SCB Transfer` reads `Bank Transfer`'s row; a disabled option is hidden
//   unless selected; the row's `instruction` is shown; `account` is filled from the row when empty.
// - The net shown is the quote's line `settleThb`, as answered (D-4). Design §6 and #543 say `store.quote().net`; the
//   generated `QuoteDto` has no `net`, so the line's `settleThb` is the server's net (see the preparer's comment).
// - Submit: refusals are shown and nothing is sent; for Cash, v2's count confirm (desk-devices.js:4) then
//   `setCountConfirmed(true)`; then `store.submit()` once and `go(4)`.
// The store is a stub with §5.3's names, so this file does not depend on `desk-store.ts`.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";
import type { DeskPaymentMethod } from "../../../../shared/api/generated/desk_config";
import type { QuoteDto } from "../../../../shared/api/generated/quote";

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
  expect(l, `web/src/screens/desk/steps/payment/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const HEADING = "ชำระเงินและเอกสารบัญชี";
const CANCEL = "ยกเลิกการจอง / แก้ไข";
const SUBMIT = "ยืนยันและบันทึกบัญชี →";
const INCOMPLETE = "ตรวจชื่อบัญชี เลขอ้างอิง สลิป พาสปอร์ต และยอดสุทธิให้ครบ";
const SLIP_BAD = "สลิปต้องเป็น JPG, PNG, WebP หรือ PDF ไม่เกิน 15 MB";
const CHANNEL_OFF = "ช่องทางนี้ปิดใช้งานสำหรับรายการนี้ กรุณาตรวจสอบกับผู้ดูแล";
const NO_RULES = "ยังโหลดการตั้งค่าวิธีชำระเงินไม่ได้ กรุณาโหลดหน้าทำรายการใหม่";
const COUNT_CONFIRM_START = "ยืนยันว่าได้ตรวจสกุล ชนิดราคา และจำนวนธนบัตรก่อนส่งมอบแล้ว?";
const COUNT_NOT_CONFIRMED = "ยังไม่ยืนยันการนับเงิน";
const CANCEL_CONFIRM = "ยกเลิกการจองนี้และกลับไปแก้ไขเรท?";
const V2_METHODS = ["Credit Card", "Alipay", "PromptPay", "Cash", "Bank Transfer", "SCB Transfer"];

const PATHS = { OWNER_DESK: { kind: "owner" }, STAFF_DESK: { kind: "staff" } } as const;

const rule = (id: string, over: Partial<DeskPaymentMethod> = {}): DeskPaymentMethod => ({
  id,
  enabled: true,
  buy: true,
  sell: true,
  account: "",
  instruction: "",
  ...over,
});

const RULES: DeskPaymentMethod[] = [
  rule("Cash", { account: "Cash drawer", instruction: "นับเงินต่อหน้าลูกค้า" }),
  rule("Bank Transfer", { account: "SCB 123-4-56789-0", instruction: "โอนเข้าบัญชีร้าน" }),
  rule("PromptPay", { buy: false }),
  rule("Credit Card", { enabled: false }),
  rule("Alipay", { sell: false }),
];

function quote(settleThb: string): QuoteDto {
  return {
    id: "q-1",
    type: "BUY",
    channel: "DESK",
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: { id: "p", version: 1 },
    lines: [
      {
        currency: "USD",
        shopSide: "BUY",
        quantity: "100",
        unitSize: "1",
        baseRate: "35.12000000",
        finalRate: "35.12000000",
        thbAmount: "3512.00",
        feeThb: "0.00",
        settleThb,
        publishedRateLineId: "l",
      },
    ],
    benefits: [],
    stale: false,
  };
}

type Init = {
  rules?: DeskPaymentMethod[] | null;
  type?: "BUY" | "SELL";
  method?: string;
  ref?: string;
  account?: string;
  slip?: File;
  checked?: boolean;
  entry?: unknown;
};

function stubStore(path: unknown, init: Init = {}) {
  const [config] = createSignal<any>(init.rules === null ? undefined : { shiftOpen: true, paymentMethods: init.rules ?? RULES });
  const [type] = createSignal(init.type ?? "BUY");
  const [q] = createSignal<QuoteDto | undefined>(quote("3512.00"));
  const [method, setMethod] = createSignal(init.method ?? "Cash");
  const [ref, setRef] = createSignal(init.ref ?? "");
  const [account, setAccount] = createSignal(init.account ?? "");
  const [fee, setFee] = createSignal("");
  const [bankFee, setBankFee] = createSignal("");
  const [notes, setNotes] = createSignal("");
  const [slip, setSlip] = createSignal<File | undefined>(init.slip);
  const [paymentChecked, setPaymentChecked] = createSignal(init.checked ?? false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const [step] = createSignal(3);
  const order: string[] = [];
  const store = {
    path,
    step,
    config,
    configError: () => undefined,
    type,
    quote: q,
    quoting: () => false,
    passport: () => new File([new Uint8Array([1])], "p.jpg", { type: "image/jpeg" }),
    method,
    ref,
    account,
    fee,
    bankFee,
    notes,
    slip,
    paymentChecked,
    countConfirmed,
    entry: () => init.entry,
    outcome: () => "idle",
    saveError: () => undefined,
    setMethod: vi.fn((v: string) => setMethod(v)),
    setRef: vi.fn((v: string) => setRef(v)),
    setAccount: vi.fn((v: string) => setAccount(v)),
    setFee: vi.fn((v: string) => setFee(v)),
    setBankFee: vi.fn((v: string) => setBankFee(v)),
    setNotes: vi.fn((v: string) => setNotes(v)),
    setSlip: vi.fn((v: File | undefined) => setSlip(v)),
    setPaymentChecked: vi.fn((v: boolean) => setPaymentChecked(v)),
    setCountConfirmed: vi.fn((v: boolean) => {
      order.push(`countConfirmed=${v}`);
      setCountConfirmed(v);
    }),
    submit: vi.fn(async () => {
      order.push("submit");
      return "saved";
    }),
    backToEdit: vi.fn(),
    go: vi.fn((i: number) => order.push(`go(${i})`)),
    _order: order,
  };
  return store;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let confirmSpy: ReturnType<typeof vi.fn>;
beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("step 3 must not call the network itself"))));
  confirmSpy = vi.fn(() => true);
  vi.stubGlobal("confirm", confirmSpy);
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
  await Promise.resolve();
  return step;
}

const control = <T extends Element = HTMLInputElement>(name: string) => host.querySelector<T & HTMLElement>(`[name="${name}"]`);
const select = () => control<HTMLSelectElement>("method")!;
const option = (v: string) => [...select().options].find((o) => o.value === v);
const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find((b) => (b.textContent ?? "").trim() === text);
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};

function choose(v: string) {
  select().value = v;
  select().dispatchEvent(new Event("change", { bubbles: true }));
}

function type(name: string, value: string) {
  const el = control(name)!;
  (el as HTMLInputElement).value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
}

function check(name: string, value = true) {
  const box = control<HTMLInputElement>(name)!;
  if (box.checked !== value) box.click();
}

describe("steps/payment: the module", () => {
  it("P0 exports step 3 with a label from its own i18n table", async () => {
    const { step } = await load(stepMods, "./step.tsx");
    expect(step.index).toBe(3);
    expect(step.label().trim()).not.toBe("");
  });

  it("P0b i18n.ts has th and en with the same keys; th carries v2's step-3 copy verbatim", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    expect((th as Record<string, unknown>).zh).toBeUndefined();
    const values = Object.values(th);
    for (const s of [HEADING, CANCEL, SUBMIT, INCOMPLETE, SLIP_BAD, CHANNEL_OFF, NO_RULES, COUNT_NOT_CONFIRMED, CANCEL_CONFIRM]) {
      expect(values, s).toContain(s);
    }
    expect(values.some((v) => v.startsWith(COUNT_CONFIRM_START)), COUNT_CONFIRM_START).toBe(true);
  });

  it("P0c no money arithmetic and no fetch( in the step's sources (W7)", () => {
    expect(Object.keys(SOURCES)).toContain("./step.tsx");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`steps/payment on ${which}`, () => {
    it("P1 v2's heading, the six methods in v2's order, and the quote's net exactly as answered", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      expect(host.textContent).toContain(HEADING);
      expect([...select().options].map((o) => o.value)).toEqual(V2_METHODS);
      expect(host.textContent, "the server's settleThb, unchanged").toContain("3512.00");
    });

    it("P2 channel filter (D-7): enabled + the type's flag; a disabled option is hidden unless selected", async () => {
      const store = stubStore(PATHS[which], { type: "BUY", method: "Cash" });
      await mount(store);
      expect(option("Cash")!.disabled).toBe(false);
      expect(option("Bank Transfer")!.disabled).toBe(false);
      expect(option("PromptPay")!.disabled, "buy flag off").toBe(true);
      expect(option("PromptPay")!.hidden).toBe(true);
      expect(option("Credit Card")!.disabled, "row disabled").toBe(true);
      expect(option("Alipay")!.disabled, "sell off, but this is BUY").toBe(false);
    });

    it("P3 the SELL direction reads each row's sell flag", async () => {
      const store = stubStore(PATHS[which], { type: "SELL", method: "Cash" });
      await mount(store);
      expect(option("Alipay")!.disabled, "sell flag off").toBe(true);
      expect(option("PromptPay")!.disabled, "buy off, but this is SELL").toBe(false);
    });

    it("P4 SCB Transfer reads Bank Transfer's row", async () => {
      const store = stubStore(PATHS[which], {
        method: "Cash",
        rules: [rule("Cash"), rule("Bank Transfer", { enabled: false })],
      });
      await mount(store);
      expect(option("SCB Transfer")!.disabled).toBe(true);
      dispose?.();
      host.innerHTML = "";
      const on = stubStore(PATHS[which], { method: "Cash", rules: [rule("Cash"), rule("Bank Transfer")] });
      await mount(on);
      expect(option("SCB Transfer")!.disabled).toBe(false);
    });

    it("P5 the chosen row's instruction is shown, and an empty account is filled from the row", async () => {
      const store = stubStore(PATHS[which], { method: "Cash", account: "" });
      await mount(store);
      choose("Bank Transfer");
      await settle();
      expect(store.setMethod).toHaveBeenCalledWith("Bank Transfer");
      expect(host.textContent).toContain("โอนเข้าบัญชีร้าน");
      expect(store.setAccount).toHaveBeenCalledWith("SCB 123-4-56789-0");
    });

    it("P6 a typed account is not overwritten by the row's", async () => {
      const store = stubStore(PATHS[which], { method: "Cash", account: "my own account" });
      await mount(store);
      choose("Bank Transfer");
      await settle();
      expect(store.setAccount).not.toHaveBeenCalledWith("SCB 123-4-56789-0");
    });

    it("P7 a method that is off for this direction: the notice, the submit disabled, and nothing sent", async () => {
      const store = stubStore(PATHS[which], { method: "Credit Card", ref: "R1", account: "A", checked: true });
      await mount(store);
      expect(host.textContent).toContain(CHANNEL_OFF);
      expect(submit().disabled).toBe(true);
      submit().form?.requestSubmit?.();
      submit().form?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      await settle();
      expect(store.submit).not.toHaveBeenCalled();
    });

    it("P8 no payment rules loaded: v2's load notice", async () => {
      const store = stubStore(PATHS[which], { rules: null });
      await mount(store);
      expect(host.textContent).toContain(NO_RULES);
    });

    it("P9 Cash: the reference is disabled", async () => {
      const store = stubStore(PATHS[which], { method: "Cash" });
      await mount(store);
      expect(control("ref")!.hasAttribute("disabled")).toBe(true);
    });

    it("P10 the submit is disabled until the payment box is checked; the box's label carries the net", async () => {
      const store = stubStore(PATHS[which], { method: "Cash", account: "Cash drawer" });
      await mount(store);
      expect(submit().disabled).toBe(true);
      expect(control("paymentChecked")!.closest("label")?.textContent ?? host.textContent).toContain("3512.00");
      check("paymentChecked");
      expect(store.setPaymentChecked).toHaveBeenCalledWith(true);
      expect(submit().disabled).toBe(false);
    });

    it("P11 a fee edit goes to setFee as typed (the store re-quotes and clears the box, D-5, dh:117)", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      type("fee", "25.50");
      expect(store.setFee).toHaveBeenLastCalledWith("25.50");
      type("bankFee", "10");
      expect(store.setBankFee).toHaveBeenLastCalledWith("10");
    });

    it("P12 non-cash without a reference or a slip, or with no account: v2's refusal and nothing sent", async () => {
      for (const init of [
        { method: "Bank Transfer", ref: "", account: "A", slip: new File([new Uint8Array([1])], "s.png", { type: "image/png" }) },
        { method: "Bank Transfer", ref: "R1", account: "A" },
        { method: "Cash", ref: "", account: "" },
      ]) {
        const store = stubStore(PATHS[which], { ...init, checked: true });
        await mount(store);
        submit().click();
        await settle();
        expect(host.textContent, JSON.stringify({ ...init, slip: !!init.slip })).toContain(INCOMPLETE);
        expect(store.submit).not.toHaveBeenCalled();
        dispose?.();
        host.innerHTML = "";
      }
    });

    it("P13 a slip of the wrong type or over 15 MB is refused with v2's sentence and not kept", async () => {
      for (const f of [
        new File([new Uint8Array([1])], "s.heic", { type: "image/heic" }),
        new File([new Uint8Array(15 * 1024 * 1024 + 1)], "big.png", { type: "image/png" }),
      ]) {
        const store = stubStore(PATHS[which], { method: "Bank Transfer" });
        await mount(store);
        const input = control<HTMLInputElement>("slip")!;
        Object.defineProperty(input, "files", { value: [f], configurable: true });
        input.dispatchEvent(new Event("change", { bubbles: true }));
        await settle();
        expect(host.textContent, f.name).toContain(SLIP_BAD);
        expect(store.setSlip, f.name).not.toHaveBeenCalledWith(f);
        dispose?.();
        host.innerHTML = "";
      }
    });

    it("P14 Cash submit: v2's count confirm, then setCountConfirmed(true), the reference sent as —, submit once, go(4)", async () => {
      const store = stubStore(PATHS[which], { method: "Cash", account: "Cash drawer", checked: true });
      await mount(store);
      submit().click();
      await settle();
      expect(confirmSpy).toHaveBeenCalledTimes(1);
      expect(String(confirmSpy.mock.calls[0]![0])).toMatch(new RegExp(`^${COUNT_CONFIRM_START.replace(/[?.]/g, "\\$&")}`));
      expect(store.setRef).toHaveBeenCalledWith("—");
      expect(store.submit).toHaveBeenCalledTimes(1);
      expect(store._order.indexOf("countConfirmed=true")).toBeLessThan(store._order.indexOf("submit"));
      expect(store.go).toHaveBeenCalledWith(4);
    });

    it("P15 Cash count not confirmed: v2's sentence, nothing sent", async () => {
      confirmSpy.mockReturnValue(false);
      const store = stubStore(PATHS[which], { method: "Cash", account: "Cash drawer", checked: true });
      await mount(store);
      submit().click();
      await settle();
      expect(host.textContent).toContain(COUNT_NOT_CONFIRMED);
      expect(store.submit).not.toHaveBeenCalled();
    });

    it("P16 non-cash with reference, slip and account: no count confirm, submit once, go(4)", async () => {
      const store = stubStore(PATHS[which], {
        method: "Bank Transfer",
        ref: "R1",
        account: "SCB 123-4-56789-0",
        slip: new File([new Uint8Array([1])], "s.png", { type: "image/png" }),
        checked: true,
      });
      await mount(store);
      submit().click();
      await settle();
      expect(confirmSpy).not.toHaveBeenCalled();
      expect(store.submit).toHaveBeenCalledTimes(1);
      expect(store.go).toHaveBeenCalledWith(4);
    });

    it("P17 cancel asks v2's question; yes goes back to step 0, no stays", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      confirmSpy.mockReturnValueOnce(false);
      button(CANCEL)!.click();
      expect(store.go).not.toHaveBeenCalledWith(0);
      expect(store.backToEdit).not.toHaveBeenCalled();
      button(CANCEL)!.click();
      expect(confirmSpy).toHaveBeenLastCalledWith(CANCEL_CONFIRM);
      expect(store.go.mock.calls.some((c) => c[0] === 0) || store.backToEdit.mock.calls.length > 0).toBe(true);
    });

    it("P18 an entry's identity checkbox (B2) carries its label and writes to the entry", async () => {
      const [confirmed, setConfirmed] = createSignal(false);
      const entry = {
        kind: "kiosk",
        id: "b-1",
        banner: () => "",
        locked: true,
        identityLabel: "ตรวจตัวตนลูกค้าตรงกับผู้จอง JANE แล้ว",
        identityConfirmed: confirmed,
        setIdentityConfirmed: vi.fn((v: boolean) => setConfirmed(v)),
        payloadExtras: () => ({}),
        refusal: () => undefined,
      };
      const store = stubStore(PATHS[which], { entry });
      await mount(store);
      expect(host.textContent).toContain("ตรวจตัวตนลูกค้าตรงกับผู้จอง JANE แล้ว");
      check("identityConfirmed");
      expect(entry.setIdentityConfirmed).toHaveBeenCalledWith(true);
    });

    it("P19 English follows the language", async () => {
      setLang("en");
      const store = stubStore(PATHS[which]);
      await mount(store);
      expect(host.textContent).not.toContain(HEADING);
    });
  });
}
