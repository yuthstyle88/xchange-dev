// B03 the desk's `?kiosk=<id>` entry (FE-desk, #564; design xchange-v3-wave3-desk.md §3.6 row K1, §5.3 `DeskEntryState`;
// kiosk actions `get` / `identityChecked` at api/src/presentation/http/kiosk.rs:630-663, answer shapes at
// api/src/presentation/dto/kiosk.rs:203-216 and :378-402). Written before the implementation by the FE-desk
// sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests:
// - `entry: DeskEntry` from `./entry` with `param: 'kiosk'`.
// - `load(store, id, api)` sends one `POST /api/kiosk` `{ action: 'get', payload: { id, branchId: store.branch() } }`,
//   then pre-fills the store: `setType(type)`, `setCurrency(currency)`, `setAmount(String(foreign))` and
//   `setMethod(paymentMethod)` (the kiosk's method names are the desk's, `steps/payment/channels.ts:5`).
// - The state it resolves: `kind: 'kiosk'`, `id`, `locked: true` (step 0's fields read-only), `banner()` naming the
//   queue (`./i18n.ts` `banner` with `{queue}`), an `identityLabel`, `identityConfirmed` / `setIdentityConfirmed`
//   starting false, `payloadExtras()` = `{ kioskRequestId: id }`, `refusal()` = a text while the identity is not
//   confirmed and `undefined` after, `beforeReserve(store)` sending one `POST /api/kiosk`
//   `{ action: 'identityChecked', payload: { id, branchId, confirmed: store.identityChecked() } }`, and no `afterSave`
//   (the receipt publish waits on O-1).
// - A refused `get` rejects `load` (the desk shows no entry).
import { describe, expect, it, vi } from "vitest";
import { ApiClient } from "../../../../shared/api/client";
import { entry } from "./entry";

const GET_ANSWER = {
  paymentMethod: "Bank Transfer",
  type: "SELL",
  currency: "USD",
  rate: 32.1,
  foreign: 500,
  thb: 16050,
  inputAmount: 500,
  branchName: "XC-B",
  id: "k-42",
  queue: "K042",
};

function fakeServer(get: () => { status: number; body: unknown } = () => ({ status: 200, body: GET_ANSWER })) {
  const calls: { path: string; body: any }[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split("?")[0];
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ path, body });
    const a =
      path === "/api/kiosk" && body?.action === "get"
        ? get()
        : path === "/api/kiosk" && body?.action === "identityChecked"
          ? { status: 200, body: { id: "k-42", queue: "K042", orderNo: "1", transactionNo: null, status: "ID_CHECKED" } }
          : { status: 404, body: { error: { code: "notFound", message: "nf" } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { "content-type": "application/json" } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

function stubStore(api: ApiClient, identityChecked = true) {
  return {
    api,
    branch: () => "MAIN",
    identityChecked: () => identityChecked,
    setType: vi.fn(),
    setCurrency: vi.fn(),
    setAmount: vi.fn(),
    setMethod: vi.fn(),
  };
}

describe("the kiosk entry", () => {
  it("E1 is the kiosk param", () => {
    expect(entry.param).toBe("kiosk");
  });

  it("E2 load reads the request once and pre-fills type, currency, amount and method", async () => {
    const server = fakeServer();
    const store = stubStore(server.api);
    await entry.load(store as any, "k-42", server.api);
    const gets = server.calls.filter((c) => c.body?.action === "get");
    expect(gets).toHaveLength(1);
    expect(gets[0].body.payload).toEqual({ id: "k-42", branchId: "MAIN" });
    expect(store.setType).toHaveBeenCalledWith("SELL");
    expect(store.setCurrency).toHaveBeenCalledWith("USD");
    expect(store.setAmount).toHaveBeenCalledWith("500");
    expect(store.setMethod).toHaveBeenCalledWith("Bank Transfer");
  });

  it("E3 the state is a locked kiosk entry naming its queue, with the request id on the save", async () => {
    const server = fakeServer();
    const state = await entry.load(stubStore(server.api) as any, "k-42", server.api);
    expect(state.kind).toBe("kiosk");
    expect(state.id).toBe("k-42");
    expect(state.locked).toBe(true);
    expect(state.banner()).toContain("K042");
    expect(state.payloadExtras()).toEqual({ kioskRequestId: "k-42" });
    expect(state.afterSave).toBeUndefined();
  });

  it("E4 the identity tick starts false and the save is refused until it is set", async () => {
    const server = fakeServer();
    const state = await entry.load(stubStore(server.api) as any, "k-42", server.api);
    expect(typeof state.identityLabel).toBe("string");
    expect(state.identityLabel).not.toBe("");
    expect(state.identityConfirmed()).toBe(false);
    expect(state.refusal()).toBeTruthy();
    state.setIdentityConfirmed(true);
    expect(state.identityConfirmed()).toBe(true);
    expect(state.refusal()).toBeUndefined();
  });

  it("E5 beforeReserve sends identityChecked once with the store's identity tick", async () => {
    const server = fakeServer();
    const store = stubStore(server.api, true);
    const state = await entry.load(store as any, "k-42", server.api);
    await state.beforeReserve!(store as any);
    const checks = server.calls.filter((c) => c.body?.action === "identityChecked");
    expect(checks).toHaveLength(1);
    expect(checks[0].body.payload).toEqual({ id: "k-42", branchId: "MAIN", confirmed: true });
  });

  it("E6 a refused get rejects load", async () => {
    const server = fakeServer(() => ({ status: 404, body: { error: { code: "notFound", message: "nf" } } }));
    await expect(entry.load(stubStore(server.api) as any, "k-42", server.api)).rejects.toBeTruthy();
  });
});
