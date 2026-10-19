// B03 desk phone capture + SMS code (FE-desk, #564; design docs/design/blueprint/membership-points.md §6.3 CU-P1, owner
// P10 "ยืนยันเบอร์ด้วย OTP ที่เคาน์เตอร์"; commands customer_phone_start.rs / customer_phone_confirm.rs). Written before the
// implementation by the FE-desk sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests; add your
// own in another file.
//
// [required] by these tests:
// - `PhoneVerify({ api, branch, customerId, onVerified })` from `./PhoneVerify`.
// - `input[name=phone]` and `button[name=sendCode]`: a blank phone sends nothing; otherwise one
//   `POST /api/customers/{id}/phone-verifications` with an Idempotency-Key and body `{ id, branch, phone }` (phone
//   trimmed; the server normalises it, §6.3 "Phone format").
// - After the start answers, `input[name=code]` and `button[name=confirmCode]` show. Confirm sends one
//   `POST /api/customers/{id}/phone-verifications/{challengeId}/confirm` with body `{ id, challengeId, branch, code }`.
// - `verified: true` calls `onVerified()` once and shows a `role=status` element.
// - Send again after a start (a new code, §6.3 "New challenge"): a second start with a different Idempotency-Key.
// - A refusal shows a `role=alert` with the text of `./i18n.ts` key `error.<code>` for `wrongCode`, `codeExpired`,
//   `tooManyAttempts`, `rateLimited`, `phoneInUse`, `smsUnavailable`; `onVerified` is not called. The code is never
//   shown back or kept after a refusal of `tooManyAttempts` or `codeExpired` (the input is emptied).
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ApiClient } from "../../../../shared/api/client";
import { setLang } from "../../../../shared/i18n";
import { PhoneVerify } from "./PhoneVerify";
import { th } from "./i18n";

type Call = { path: string; key: string | null; body: any };
function fakeServer(confirm: (body: any) => { status: number; body: unknown } = () => ({ status: 200, body: { verified: true, identityId: "id-1" } })) {
  const calls: Call[] = [];
  let n = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split("?")[0];
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ path, key: new Headers(init?.headers).get("idempotency-key"), body });
    let a: { status: number; body: unknown };
    if (path === "/api/customers/c-1/phone-verifications") a = { status: 200, body: { challengeId: `ch-${++n}`, expiresAt: new Date(Date.now() + 300_000).toISOString() } };
    else if (/^\/api\/customers\/c-1\/phone-verifications\/ch-\d+\/confirm$/.test(path)) a = confirm(body);
    else a = { status: 404, body: { error: { code: "notFound", message: "nf" } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { "content-type": "application/json" } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang("th");
});
function mount(server: ReturnType<typeof fakeServer>, onVerified = vi.fn()) {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <PhoneVerify api={server.api} branch="MAIN" customerId="c-1" onVerified={onVerified} />, host);
  return onVerified;
}
const el = <T extends Element>(sel: string) => host.querySelector<T>(sel);
function type(sel: string, value: string) {
  const i = el<HTMLInputElement>(sel)!;
  i.value = value;
  i.dispatchEvent(new Event("input", { bubbles: true }));
}
async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}
const starts = (s: ReturnType<typeof fakeServer>) => s.calls.filter((c) => c.path === "/api/customers/c-1/phone-verifications");
const confirms = (s: ReturnType<typeof fakeServer>) => s.calls.filter((c) => c.path.endsWith("/confirm"));

async function startWith(_server: ReturnType<typeof fakeServer>, phone = " 0812345678 ") {
  type("input[name=phone]", phone);
  el<HTMLButtonElement>("button[name=sendCode]")!.click();
  await flush();
}

describe("PhoneVerify", () => {
  it("P1 a blank phone sends nothing", async () => {
    const server = fakeServer();
    mount(server);
    expect(el("input[name=phone]")).not.toBeNull();
    await startWith(server, "   ");
    expect(starts(server)).toHaveLength(0);
  });

  it("P2 send code starts one challenge with the trimmed phone and a key, then asks for the code", async () => {
    const server = fakeServer();
    mount(server);
    await startWith(server);
    expect(starts(server)).toHaveLength(1);
    expect(starts(server)[0].body).toEqual({ id: "c-1", branch: "MAIN", phone: "0812345678" });
    expect(starts(server)[0].key).toBeTruthy();
    expect(el("input[name=code]")).not.toBeNull();
    expect(el("button[name=confirmCode]")).not.toBeNull();
  });

  it("P3 confirm sends the challenge and code; verified calls onVerified once and shows a status", async () => {
    const server = fakeServer();
    const onVerified = mount(server);
    await startWith(server);
    type("input[name=code]", "123456");
    el<HTMLButtonElement>("button[name=confirmCode]")!.click();
    await flush();
    expect(confirms(server)).toHaveLength(1);
    expect(confirms(server)[0].path).toBe("/api/customers/c-1/phone-verifications/ch-1/confirm");
    expect(confirms(server)[0].body).toEqual({ id: "c-1", challengeId: "ch-1", branch: "MAIN", code: "123456" });
    expect(onVerified).toHaveBeenCalledTimes(1);
    expect(el("[role=status]")).not.toBeNull();
  });

  it("P4 send again starts a new challenge with a new key, and confirm uses the newest", async () => {
    const server = fakeServer();
    mount(server);
    await startWith(server);
    el<HTMLButtonElement>("button[name=sendCode]")!.click();
    await flush();
    expect(starts(server)).toHaveLength(2);
    expect(starts(server)[1].key).not.toBe(starts(server)[0].key);
    type("input[name=code]", "654321");
    el<HTMLButtonElement>("button[name=confirmCode]")!.click();
    await flush();
    expect(confirms(server).at(-1)!.path).toBe("/api/customers/c-1/phone-verifications/ch-2/confirm");
  });

  it.each(["wrongCode", "codeExpired", "tooManyAttempts", "rateLimited", "phoneInUse", "smsUnavailable"])(
    "P5 a %s refusal of confirm shows its own text and does not verify",
    async (code) => {
      const server = fakeServer(() => ({ status: 409, body: { error: { code, message: "refused" } } }));
      const onVerified = mount(server);
      await startWith(server);
      type("input[name=code]", "123456");
      el<HTMLButtonElement>("button[name=confirmCode]")!.click();
      await flush();
      expect(el("[role=alert]")?.textContent ?? "").toContain((th as Record<string, string>)[`error.${code}`]);
      expect(onVerified).not.toHaveBeenCalled();
    },
  );

  it.each(["tooManyAttempts", "codeExpired"])("P6 after a %s refusal the code input is emptied", async (code) => {
    const server = fakeServer(() => ({ status: 409, body: { error: { code, message: "refused" } } }));
    mount(server);
    await startWith(server);
    type("input[name=code]", "123456");
    el<HTMLButtonElement>("button[name=confirmCode]")!.click();
    await flush();
    expect(el<HTMLInputElement>("input[name=code]")?.value ?? "").toBe("");
    expect(host.textContent ?? "").not.toContain("123456");
  });
});
