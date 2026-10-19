// B03 coupon (FE-desk, #564): a `couponCode` quote error belongs to the coupon field (coupon/CouponField.tsx), so step 0
// does not repeat it in its general error line (`steps/rate/step.tsx` `otherError`). Written before the implementation
// by the FE-desk sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests.
//
// [required]: step 0's general error line shows a quote error whose field is not `amount`, `currency` or `couponCode`
// (today it excludes only `amount` and `currency`, step.tsx:21-24); every other behaviour of step 0 is unchanged
// (`step.test.tsx`).
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";
import { step } from "./step";

function stubStore(quoteError: { field?: string; message: string }) {
  const [config] = createSignal<any>({ shiftOpen: true });
  const [qe] = createSignal<any>(quoteError);
  return {
    path: { kind: "owner" },
    config,
    configError: () => undefined,
    quote: () => undefined,
    quoteError: qe,
    quoting: () => false,
    type: () => "BUY",
    currency: () => "USD",
    amount: () => "100",
    referenceRate: () => "",
    entry: () => undefined,
    step: () => 0,
    setType: vi.fn(),
    setCurrency: vi.fn(),
    setAmount: vi.fn(),
    setReferenceRate: vi.fn(),
    requote: vi.fn(),
    go: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang("th");
});
function mount(store: ReturnType<typeof stubStore>) {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => step.component({ store: store as any }), host);
}

describe("step 0 and the coupon error", () => {
  it("R1 a couponCode quote error is not shown by step 0", () => {
    mount(stubStore({ field: "couponCode", message: "คูปองใช้ไม่ได้" }));
    expect(host.textContent ?? "").not.toContain("คูปองใช้ไม่ได้");
  });

  it("R2 a quote error on another field is still shown by step 0", () => {
    mount(stubStore({ field: "lines", message: "อย่างอื่นผิด" }));
    expect(host.textContent ?? "").toContain("อย่างอื่นผิด");
  });
});
