// B06 per-ticket actions (FE-desk, #564; design docs/design/blueprint/queue-tickets.md Q5, §4.4 "only on the teller's own
// called ticket, or by BRANCH_MANAGER", §5 routes). Written before the implementation by the FE-desk sub-controller
// claude/dell-claude-1. Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests:
// - `TicketActions({ ticket, me, commands, onDone })` from `./TicketActions` (props as in the skeleton).
// - Buttons carry `name`: `call`, `recall`, `skip`, `start`, `finish`, `cancel`; the cancel reason is
//   `input[name=cancelReason]`, sent with `button[name=cancelConfirm]`.
// - Which buttons show, per the ticket's status and whether it is the caller's (`ticket.tellerId === me.userId`) or the
//   caller holds `BranchDayOperate` (the branch manager's permission, the proxy for "or by BRANCH_MANAGER"):
//     WAITING                      → call, cancel
//     CALLED, mine or manager      → recall, skip, start, cancel
//     CALLED, another's, no manager→ skip, cancel
//     SERVING, mine or manager     → finish
//     SERVING, another's           → none
// - A button sends exactly one command through `commands` and calls `onDone()` once after it answers; while it is in
//   flight every button is disabled.
// - Cancel needs a non-blank reason (sent trimmed); a blank one sends nothing and shows an alert.
// - A refusal shows an alert (role=alert) with the refusal's own text for `notYourTicket`, `counterPaused`,
//   `noOpenShift` and `staleVersion` (from `./i18n.ts` keys `error.<code>`), else the worded error; `onDone` is not called.
import { afterEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { ApiError } from "../../../../shared/api/client";
import { setLang } from "../../../../shared/i18n";
import { TicketActions } from "./TicketActions";
import { th } from "./i18n";

const ME = { userId: "u-me", permissions: ["QueueOperate"] };
const MANAGER = { userId: "u-boss", permissions: ["QueueOperate", "BranchDayOperate"] };

function ticket(status: string, tellerId: string | null) {
  return { id: "t-1", displayNumber: "A012", status, tellerId };
}

function fakeCommands() {
  const answer = { id: "t-1", displayNumber: "A012", serviceType: "WALK_IN", sequence: 12, status: "CALLED", tellerId: "u-me", endedReason: null };
  return {
    issueWalkIn: vi.fn(async () => answer),
    callNext: vi.fn(async () => answer),
    ticket: vi.fn(async (_verb: string, _id: string) => answer),
    cancel: vi.fn(async (_id: string, _reason: string) => answer),
    counter: vi.fn(async () => ({ tellerId: "u-me", status: "ACTIVE" })),
    queue: vi.fn(async () => ({ queueOpen: true, activeCounters: 1 })),
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

const names = () => [...host.querySelectorAll("button")].map((b) => b.getAttribute("name")).filter(Boolean).sort();
const button = (n: string) => host.querySelector<HTMLButtonElement>(`button[name=${n}]`);
async function flush() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe("TicketActions: which buttons show", () => {
  const cases: [string, string, string | null, typeof ME, string[]][] = [
    ["WAITING", "WAITING", null, ME, ["call", "cancel"]],
    ["CALLED, mine", "CALLED", "u-me", ME, ["cancel", "recall", "skip", "start"]],
    ["CALLED, another's, manager", "CALLED", "u-other", MANAGER, ["cancel", "recall", "skip", "start"]],
    ["CALLED, another's, no manager", "CALLED", "u-other", ME, ["cancel", "skip"]],
    ["SERVING, mine", "SERVING", "u-me", ME, ["finish"]],
    ["SERVING, another's, manager", "SERVING", "u-other", MANAGER, ["finish"]],
    ["SERVING, another's", "SERVING", "u-other", ME, []],
  ];
  it.each(cases)("A1 %s", (_label, status, teller, me, want) => {
    const commands = fakeCommands();
    mount(() => <TicketActions ticket={ticket(status, teller)} me={me} commands={commands} onDone={() => {}} />);
    expect(names()).toEqual(want);
  });
});

describe("TicketActions: commands", () => {
  it.each([
    ["call", "WAITING", null],
    ["recall", "CALLED", "u-me"],
    ["skip", "CALLED", "u-me"],
    ["start", "CALLED", "u-me"],
    ["finish", "SERVING", "u-me"],
  ] as const)("A2 %s sends one command for the ticket, then onDone once", async (verb, status, teller) => {
    const commands = fakeCommands();
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket(status, teller)} me={ME} commands={commands} onDone={onDone} />);
    button(verb)!.click();
    await flush();
    expect(commands.ticket).toHaveBeenCalledTimes(1);
    expect(commands.ticket).toHaveBeenCalledWith(verb, "t-1");
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("A3 while a command is in flight every button is disabled and a second click sends nothing", async () => {
    const commands = fakeCommands();
    let release!: () => void;
    commands.ticket.mockImplementation(() => new Promise((r) => (release = () => r({} as never))));
    mount(() => <TicketActions ticket={ticket("CALLED", "u-me")} me={ME} commands={commands} onDone={() => {}} />);
    button("start")!.click();
    await flush();
    expect([...host.querySelectorAll("button")].every((b) => b.disabled)).toBe(true);
    button("start")!.click();
    expect(commands.ticket).toHaveBeenCalledTimes(1);
    release();
    await flush();
  });

  it("A4 cancel with a blank reason sends nothing and shows an alert", async () => {
    const commands = fakeCommands();
    mount(() => <TicketActions ticket={ticket("WAITING", null)} me={ME} commands={commands} onDone={() => {}} />);
    button("cancel")!.click();
    await flush();
    const reason = host.querySelector<HTMLInputElement>("input[name=cancelReason]");
    expect(reason).not.toBeNull();
    reason!.value = "   ";
    reason!.dispatchEvent(new Event("input", { bubbles: true }));
    button("cancelConfirm")!.click();
    await flush();
    expect(commands.cancel).not.toHaveBeenCalled();
    expect(host.querySelector("[role=alert]")).not.toBeNull();
  });

  it("A5 cancel sends the trimmed reason, then onDone once", async () => {
    const commands = fakeCommands();
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket("CALLED", "u-me")} me={ME} commands={commands} onDone={onDone} />);
    button("cancel")!.click();
    await flush();
    const reason = host.querySelector<HTMLInputElement>("input[name=cancelReason]")!;
    reason.value = "  customer left  ";
    reason.dispatchEvent(new Event("input", { bubbles: true }));
    button("cancelConfirm")!.click();
    await flush();
    expect(commands.cancel).toHaveBeenCalledWith("t-1", "customer left");
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it.each(["notYourTicket", "counterPaused", "noOpenShift", "staleVersion"])(
    "A6 a %s refusal shows its own text and does not call onDone",
    async (code) => {
      const commands = fakeCommands();
      commands.ticket.mockRejectedValue(new ApiError({ status: 409, code, message: "refused" }));
      const onDone = vi.fn();
      mount(() => <TicketActions ticket={ticket("CALLED", "u-me")} me={ME} commands={commands} onDone={onDone} />);
      button("recall")!.click();
      await flush();
      const alert = host.querySelector("[role=alert]");
      expect(alert?.textContent).toContain((th as Record<string, string>)[`error.${code}`]);
      expect(onDone).not.toHaveBeenCalled();
    },
  );
});
