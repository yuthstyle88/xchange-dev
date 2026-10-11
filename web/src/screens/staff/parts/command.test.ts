// S-3 #575: `parts/command.ts` — a CA-8 command sent through `ApiClient.command` with an `Idempotency-Key` that is
// kept until the command succeeds and resent unchanged, with a byte-equal body, on retry (SD:S5, SD:§4, §9 edge case).
// Written before the implementation by the ticket author (claude/709ec749). Do not edit, delete or weaken.
//
// [required]: export function command<B, R>(api: ApiClient, method: string, path: string): { send(body: B): Promise<R> }
// - every `send` goes through `api.command(method, path, body, key)` (so the `idempotency-key` header is set);
// - a send that fails (a refusal or a network error) keeps the key: the next `send` reuses it;
// - after a success the key is spent: the next `send` uses a new, non-empty key.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiClient } from "../../../shared/api/client";

type Sender = { send: (body: unknown) => Promise<unknown> };
type CommandModule = { command: (api: ApiClient, method: string, path: string) => Sender };
const mods = import.meta.glob<CommandModule>("./command.ts");

async function command(): Promise<CommandModule["command"]> {
  const l = mods["./command.ts"];
  expect(l, "web/src/screens/staff/parts/command.ts exists").toBeDefined();
  return (await l!()).command;
}

let answers: Array<() => Response | Promise<Response>>;
let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  answers = [];
  fetchSpy = vi.fn(async () => {
    const next = answers.shift();
    if (!next) throw new Error("no answer queued");
    return next();
  });
});

const json = (status: number, body: unknown) => () =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function sent(i: number): { url: string; key: string | null; body: string; method: string } {
  const [url, init] = fetchSpy.mock.calls[i] as [string, RequestInit];
  const headers = new Headers(init.headers);
  return {
    url,
    key: headers.get("idempotency-key"),
    body: String(init.body),
    method: String(init.method),
  };
}

describe("command(api, method, path).send(body)", () => {
  it("I1 sends method, path and body through ApiClient.command with a non-empty Idempotency-Key", async () => {
    const make = await command();
    answers.push(json(200, { ok: true }));
    const c = make(new ApiClient(fetchSpy as unknown as typeof fetch), "POST", "/api/cash/counts");
    await expect(c.send({ kind: "OPEN", lines: [{ faceValueId: 7, quantity: "3" }] })).resolves.toEqual({ ok: true });
    const s = sent(0);
    expect(s.method).toBe("POST");
    expect(s.url).toBe("/api/cash/counts");
    expect(s.key).toBeTruthy();
    expect(JSON.parse(s.body)).toEqual({ kind: "OPEN", lines: [{ faceValueId: 7, quantity: "3" }] });
  });

  it("I2 a refused send keeps the key: the retry resends the same key and a byte-equal body", async () => {
    const make = await command();
    answers.push(json(503, { error: { code: "busy", message: "busy" } }));
    answers.push(json(200, { ok: true }));
    const c = make(new ApiClient(fetchSpy as unknown as typeof fetch), "POST", "/api/cash/counts");
    const body = { kind: "CLOSE", lines: [{ faceValueId: 1, quantity: "10" }] };
    await expect(c.send(body)).rejects.toBeTruthy();
    await c.send(body);
    expect(sent(1).key).toBe(sent(0).key);
    expect(sent(1).body).toBe(sent(0).body);
  });

  it("I3 a network error keeps the key too", async () => {
    const make = await command();
    answers.push(() => {
      throw new TypeError("Failed to fetch");
    });
    answers.push(json(200, { ok: true }));
    const c = make(new ApiClient(fetchSpy as unknown as typeof fetch), "POST", "/api/cash/transfers");
    await expect(c.send({ a: "1" })).rejects.toBeTruthy();
    await c.send({ a: "1" });
    expect(sent(1).key).toBe(sent(0).key);
  });

  it("I4 after a success the key is spent: the next send uses a new key", async () => {
    const make = await command();
    answers.push(json(200, { ok: true }));
    answers.push(json(200, { ok: true }));
    const c = make(new ApiClient(fetchSpy as unknown as typeof fetch), "POST", "/api/cash/counts");
    await c.send({ n: "1" });
    await c.send({ n: "2" });
    expect(sent(1).key).toBeTruthy();
    expect(sent(1).key).not.toBe(sent(0).key);
  });

  it("I5 two commands made separately never share a key", async () => {
    const make = await command();
    answers.push(json(200, {}));
    answers.push(json(200, {}));
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    await make(api, "POST", "/api/cash/counts").send({});
    await make(api, "POST", "/api/cash/counts").send({});
    expect(sent(1).key).not.toBe(sent(0).key);
  });
});
