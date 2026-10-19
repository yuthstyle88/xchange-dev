// B06 counter bar (FE-desk, #564; design docs/design/blueprint/queue-tickets.md Q4, Q7, Q8, §4.4, §5). Written before the
// implementation by the FE-desk sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests; add your
// own in another file.
//
// [required] by these tests:
// - `CounterBar({ store, me, commands })` from `./CounterBar` (props as in the skeleton). It reads the caller's counter
//   from `store.state()?.myCounter` (`QueueBoardData`, shared/api/generated/queue_board.ts) and nothing else of the board.
// - Buttons carry `name`: `issueWalkIn`, `callNext`, `pause`, `resume`, `openQueue`, `closeQueue`.
// - `myCounter` null (no OPEN shift): `callNext`, `pause` and `resume` are absent or disabled, and a note is shown.
//   ACTIVE: `pause` shows, `resume` does not, `callNext` is enabled. PAUSED: `resume` shows, `pause` does not, and
//   `callNext` is disabled (the server refuses `counterPaused`).
// - `openQueue` / `closeQueue` show only when `me.permissions` holds `BranchDayOperate` (§4.4: BRANCH_MANAGER).
// - `issueWalkIn` and `callNext` send one command each, then `store.refresh()` once, and show the answered ticket's
//   `displayNumber` in a `role=status` element.
// - A refusal shows an alert (role=alert); for `nothingWaiting`, `queueClosed`, `noOpenShift` and `counterPaused` it is
//   the text of `./i18n.ts` key `error.<code>`.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { ApiError } from "../../../../shared/api/client";
import { setLang } from "../../../../shared/i18n";
import { CounterBar } from "./CounterBar";
import { th } from "./i18n";

const TELLER = { userId: "u-me", permissions: ["QueueOperate"] };
const MANAGER = { userId: "u-boss", permissions: ["QueueOperate", "BranchDayOperate"] };

function fakeStore(myCounter: { status: string } | null) {
  const [state] = createSignal<any>({ waiting: [], called: [], myCounter });
  return {
    state,
    error: () => undefined,
    lastUpdated: () => undefined,
    version: () => undefined,
    now: () => Date.now(),
    refresh: vi.fn(async () => {}),
  };
}

function fakeCommands() {
  const t = (n: string) => ({ id: "t-" + n, displayNumber: n, serviceType: "WALK_IN", sequence: 1, status: "CALLED", tellerId: "u-me", endedReason: null });
  return {
    issueWalkIn: vi.fn(async () => ({ ...t("A031"), status: "WAITING", tellerId: null })),
    callNext: vi.fn(async () => t("A007")),
    ticket: vi.fn(),
    cancel: vi.fn(),
    counter: vi.fn(async (verb: string) => ({ tellerId: "u-me", status: verb === "pause" ? "PAUSED" : "ACTIVE" })),
    queue: vi.fn(async (verb: string) => ({ queueOpen: verb === "open", activeCounters: 1 })),
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
function mount(el: () => JSX.Element) {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(el, host);
}
const button = (n: string) => host.querySelector<HTMLButtonElement>(`button[name=${n}]`);
const usable = (n: string) => !!button(n) && !button(n)!.disabled;
async function flush() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe("CounterBar: my counter", () => {
  it("B1 no counter (no OPEN shift): call next, pause and resume cannot be used, and a note shows", () => {
    mount(() => <CounterBar store={fakeStore(null)} me={TELLER} commands={fakeCommands()} />);
    expect(usable("callNext")).toBe(false);
    expect(usable("pause")).toBe(false);
    expect(usable("resume")).toBe(false);
    expect(button("issueWalkIn")).not.toBeNull();
    expect(host.textContent ?? "").toContain(th["noCounter" as keyof typeof th]);
  });

  it("B2 ACTIVE: pause shows, resume does not, call next is enabled", () => {
    mount(() => <CounterBar store={fakeStore({ status: "ACTIVE" })} me={TELLER} commands={fakeCommands()} />);
    expect(usable("pause")).toBe(true);
    expect(button("resume")).toBeNull();
    expect(usable("callNext")).toBe(true);
  });

  it("B3 PAUSED: resume shows, pause does not, call next is disabled", () => {
    mount(() => <CounterBar store={fakeStore({ status: "PAUSED" })} me={TELLER} commands={fakeCommands()} />);
    expect(usable("resume")).toBe(true);
    expect(button("pause")).toBeNull();
    expect(usable("callNext")).toBe(false);
  });

  it.each([
    ["pause", "ACTIVE"],
    ["resume", "PAUSED"],
  ] as const)("B4 %s sends the counter command once, then refreshes the board once", async (verb, status) => {
    const store = fakeStore({ status });
    const commands = fakeCommands();
    mount(() => <CounterBar store={store} me={TELLER} commands={commands} />);
    button(verb)!.click();
    await flush();
    expect(commands.counter).toHaveBeenCalledTimes(1);
    expect(commands.counter).toHaveBeenCalledWith(verb);
    expect(store.refresh).toHaveBeenCalledTimes(1);
  });
});

describe("CounterBar: tickets", () => {
  it.each([
    ["issueWalkIn", "A031"],
    ["callNext", "A007"],
  ] as const)("B5 %s sends one command, refreshes once and shows the ticket number", async (name, number) => {
    const store = fakeStore({ status: "ACTIVE" });
    const commands = fakeCommands();
    mount(() => <CounterBar store={store} me={TELLER} commands={commands} />);
    button(name)!.click();
    await flush();
    expect(commands[name]).toHaveBeenCalledTimes(1);
    expect(store.refresh).toHaveBeenCalledTimes(1);
    expect(host.querySelector("[role=status]")?.textContent ?? "").toContain(number);
  });

  it.each(["nothingWaiting", "queueClosed", "noOpenShift", "counterPaused"])(
    "B6 a %s refusal of call next shows its own text",
    async (code) => {
      const commands = fakeCommands();
      commands.callNext.mockRejectedValue(new ApiError({ status: 409, code, message: "refused" }));
      mount(() => <CounterBar store={fakeStore({ status: "ACTIVE" })} me={TELLER} commands={commands} />);
      button("callNext")!.click();
      await flush();
      expect(host.querySelector("[role=alert]")?.textContent ?? "").toContain(
        (th as Record<string, string>)[`error.${code}`],
      );
    },
  );
});

describe("CounterBar: the branch queue", () => {
  it("B7 a teller without BranchDayOperate sees no open or close", () => {
    mount(() => <CounterBar store={fakeStore({ status: "ACTIVE" })} me={TELLER} commands={fakeCommands()} />);
    expect(button("openQueue")).toBeNull();
    expect(button("closeQueue")).toBeNull();
  });

  it.each([
    ["openQueue", "open"],
    ["closeQueue", "close"],
  ] as const)("B8 a manager's %s sends queue(%s) once", async (name, verb) => {
    const commands = fakeCommands();
    mount(() => <CounterBar store={fakeStore({ status: "ACTIVE" })} me={MANAGER} commands={commands} />);
    button(name)!.click();
    await flush();
    expect(commands.queue).toHaveBeenCalledTimes(1);
    expect(commands.queue).toHaveBeenCalledWith(verb);
  });
});
