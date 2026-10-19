// B03 the desk's coupon input (FE-desk, #564; design xchange-v3-wave3-desk.md D-5, marketing-campaigns.md D-9, §10).
// Written before the implementation by the FE-desk sub-controller claude/dell-claude-1. Do not edit, delete or weaken
// these tests; add your own in another file.
//
// [required] by these tests:
// - `CouponField({ store })` from `./CouponField`; it reads `couponCode`, `quoteError` and `entry` and writes
//   `setCouponCode` only (the store re-quotes; the field never calls the network).
// - `input[name=couponCode]` shows `store.couponCode()`; each input event calls `setCouponCode` with the value as typed.
// - A kiosk entry that is `locked` makes the input read-only (the same rule as step 0's fields).
// - `quoteError()` with `field: "couponCode"` shows its message in a `role=alert` element inside the field; a quote error
//   on any other field shows nothing here.
// - `./i18n.ts` has `th` and `en` with the same keys.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../shared/i18n";
import { CouponField } from "./CouponField";
import { en, th } from "./i18n";

function stubStore(init: { coupon?: string; locked?: boolean; quoteError?: { field?: string; message: string } } = {}) {
  const [couponCode, setCoupon] = createSignal(init.coupon ?? "");
  const [quoteError, setQuoteError] = createSignal(init.quoteError);
  const [entry] = createSignal<any>(init.locked ? { kind: "kiosk", id: "k1", locked: true } : undefined);
  return {
    couponCode,
    setCouponCode: vi.fn((c: string) => setCoupon(c)),
    quoteError,
    entry,
    _setQuoteError: setQuoteError,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang("th");
  vi.unstubAllGlobals();
});
function mount(store: ReturnType<typeof stubStore>) {
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("the coupon field must not call the network"))));
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <CouponField store={store as any} />, host);
}
const input = () => host.querySelector<HTMLInputElement>("input[name=couponCode]");

describe("CouponField", () => {
  it("K1 shows the store's coupon and sends each edit as typed", () => {
    const store = stubStore({ coupon: "SAVE10" });
    mount(store);
    expect(input()?.value).toBe("SAVE10");
    input()!.value = "SAVE1";
    input()!.dispatchEvent(new Event("input", { bubbles: true }));
    expect(store.setCouponCode).toHaveBeenCalledWith("SAVE1");
  });

  it("K2 a locked entry makes the input read-only", () => {
    mount(stubStore({ locked: true }));
    expect(input()?.readOnly).toBe(true);
  });

  it("K3 a couponCode quote error shows inside the field; another field's does not", () => {
    const store = stubStore({ quoteError: { field: "couponCode", message: "คูปองใช้ไม่ได้" } });
    mount(store);
    expect(host.querySelector("[role=alert]")?.textContent ?? "").toContain("คูปองใช้ไม่ได้");
    store._setQuoteError({ field: "amount", message: "too big" });
    expect(host.querySelector("[role=alert]")).toBeNull();
    expect(host.textContent ?? "").not.toContain("too big");
  });

  it("K4 Thai and English texts have the same keys", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
  });
});
