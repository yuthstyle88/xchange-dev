// OL-1R-a (#6360 follow-up, docs/design/xchange-one-look.md §2.2, §2.4 G3): the floating queue card renders through
// <Portal> into body, outside AppShell's .xc-theme root, so the card's own root must carry xc-theme or every
// var(--xc-…) in queue-card.css is undefined. Written before the implementation by the sub-controller claude/1cc51615.
// Do not edit, delete or weaken. The setup below is copied from QueueCard.float.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Accessor } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../i18n";
import {
  queueStore,
  type QueueError,
  type QueueStore,
} from "../queue/queue-store";
import {
  makeQueueBoardData,
  makeQueueBoardWaiting,
  type Me,
  type QueueBoardData,
} from "../testing/fixtures";
import { QueueCard } from "./QueueCard";
import { meOf } from "../../test-support/me";

// ── fake queue store (as QueueCard.test.tsx's) ───────────────────────────────

interface FakeStore extends Omit<QueueStore, "state"> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({ stores: new Map<string, unknown>() }));

vi.mock("../queue/queue-store", async () => {
  const solid = await import("solid-js");
  const [branch, setBranch] = solid.createSignal("B1");
  const store = (b: string): FakeStore => {
    let s = fake.stores.get(b) as FakeStore | undefined;
    if (!s) {
      const [state, setState] = solid.createSignal<QueueBoardData | undefined>(
        undefined,
      );
      const [error, setError] = solid.createSignal<QueueError | undefined>(
        undefined,
      );
      const [lastUpdated] = solid.createSignal<number | undefined>(undefined);
      const [version] = solid.createSignal<number | undefined>(undefined);
      s = {
        state,
        error,
        lastUpdated,
        version,
        now: () => Date.now(),
        async refresh() {},
        setState: (v) => setState(() => v),
        setError: (v) => setError(() => v),
      };
      fake.stores.set(b, s);
    }
    return s;
  };
  return {
    queueBranch: branch,
    setQueueBranch: (b: string) => setBranch(b),
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
  };
});

const NOK_V3 = {
  id: "E7",
  role: "FRONT",
  branch: "B1",
  name: "Nok",
  branchName: "B1",
  branches: [],
};
const NOK: Me = meOf(NOK_V3);

/**
 * `count` WAITING tickets in call order (`queue_board`'s `waiting`); the newest one (the latest `issuedAt`) has the id
 * `latest`, so it is the card's "new request" key. The older ones keep the same ids from one answer to the next.
 */
function waiting(count: number, latest: string | null) {
  const list = [];
  for (let i = 1; i <= count; i += 1) {
    const newest = i === count;
    list.push(
      makeQueueBoardWaiting({
        id: newest && latest !== null ? latest : `older-${i}`,
        displayNumber: `W${String(i).padStart(3, "0")}`,
        serviceType: "WALK_IN",
        sequence: i,
        issuedAt: new Date(
          Date.parse("2026-10-08T02:00:00.000Z") + i * 60_000,
        ).toISOString(),
      }),
    );
  }
  return list;
}

function answer(
  count: number,
  latest: string | null = count ? `K${count}` : null,
) {
  const s = queueStore("B1") as FakeStore;
  s.setError(undefined);
  const data = makeQueueBoardData({
    waiting: waiting(count, latest),
    called: [],
    myCounter: null,
  });
  s.setState(data);
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));
let disposers: (() => void)[] = [];

async function mountCard() {
  const host = document.createElement("div");
  document.body.append(host);
  const session = { me: () => NOK, signOut: async () => {}, dispose: () => {} };
  disposers.push(render(() => <QueueCard session={session} />, host));
  await flush();
}

function card(): HTMLElement {
  const all = [...document.querySelectorAll<HTMLElement>("aside")].filter(
    (a) => a.getAttribute("aria-label") === "แจ้งเตือนคิวลูกค้า",
  );
  expect(all, "one card labelled แจ้งเตือนคิวลูกค้า").toHaveLength(1);
  return all[0]!;
}

function fold(): HTMLButtonElement {
  const b = card().querySelector<HTMLButtonElement>("button[aria-expanded]");
  expect(b, "the fold button").not.toBeNull();
  return b!;
}

beforeEach(() => {
  setLang("th");
  fake.stores.clear();
  Reflect.deleteProperty(window, "matchMedia");
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response("{}", { status: 500 })),
  );
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  Reflect.deleteProperty(window, "matchMedia");
});

// ── the classes the CSS keys on (QF7) ────────────────────────────────────────

describe("the card carries the one-look root (OL-1R-a)", () => {
  it("T1: the mounted card is aside.queue-card with xc-theme on the same element", async () => {
    await mountCard();
    expect(card().classList.contains("queue-card"), "class queue-card").toBe(
      true,
    );
    expect(
      card().classList.contains("xc-theme"),
      "class xc-theme on the portal root",
    ).toBe(true);
  });

  it("T2: xc-theme stays when the card folds and when a customer waits", async () => {
    await mountCard();
    answer(1);
    await flush();
    expect(card().classList.contains("waiting"), "waiting").toBe(true);
    expect(
      card().classList.contains("xc-theme"),
      "xc-theme next to waiting",
    ).toBe(true);
    fold().click();
    await flush();
    expect(card().classList.contains("minimized"), "minimized").toBe(true);
    expect(
      card().classList.contains("xc-theme"),
      "xc-theme next to minimized",
    ).toBe(true);
  });
});
