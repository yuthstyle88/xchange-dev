// XC Counter kiosk exit (docs/design/xc-counter-app.md §8.8, xchange#6569): the desk's side.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//
// | # | Case | Required |
// |---|---|---|
// | U1 | inside the app (`window.xcCounter`) | a link to `tauri://localhost/exit.html` |
// | U2 | a plain browser | no exit link |
// | U3 | `canIssue` false | no issue panel, no read of the view |
// | U4 | `canIssue`, panel opened | reads `branch_counters?branch=`; one option per counter |
// | U5 | issue | `counterUnlockIssue {agentId}` with an idempotency key; the answered code is shown |
// | U6 | a refusal | `wordedError` in `role="alert"`, no code shown |
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { makeBranchCounterRow } from '../../shared/testing/fixtures/branch_counters';
import { makeCounterUnlockIssueOutput } from '../../shared/testing/fixtures/counter_unlock_issue';
import { CounterUnlock } from './CounterUnlock';

const VIEW = '/api/v3/views/branch_counters';
const ISSUE = '/api/v3/commands/counter_unlock_issue';

interface Call {
  url: string;
  method: string;
  body: unknown;
  key: string | null;
}

let calls: Call[];
let answerIssue: () => Response;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn: Fetch = async (input, init) => {
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url: input, method, body, key: new Headers(init?.headers).get('idempotency-key') });
  const u = new URL(input, 'http://localhost');
  if (u.pathname === VIEW) {
    return json({
      version: 1,
      changed: true,
      data: {
        counters: [
          makeBranchCounterRow({ agentId: 'agent-1', name: 'เคาน์เตอร์ 1' }),
          makeBranchCounterRow({ agentId: 'agent-2', name: 'เคาน์เตอร์ 2' }),
        ],
        cashFromBrowser: null,
        cashFromBrowserVersion: 0,
      },
    });
  }
  if (u.pathname === ISSUE) return answerIssue();
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  calls = [];
  answerIssue = () =>
    json(makeCounterUnlockIssueOutput({ code: 'ABCDE-FGHJK', expiresAt: '2026-11-10T08:01:00Z' }));
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  delete (window as { xcCounter?: unknown }).xcCounter;
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 20; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

async function mount(canIssue: boolean) {
  dispose = render(
    () => <CounterUnlock branchId="MAIN" canIssue={canIssue} api={new ApiClient(fetchFn)} />,
    host,
  );
  await settle();
}

async function openPanel(): Promise<HTMLDetailsElement> {
  const details = host.querySelector<HTMLDetailsElement>('[data-counter-unlock]');
  expect(details, 'the issue panel').not.toBeNull();
  details!.open = true;
  details!.dispatchEvent(new Event('toggle'));
  await settle();
  return details!;
}

describe('CounterUnlock', () => {
  it('U1: inside the app it links to the exit page', async () => {
    (window as { xcCounter?: unknown }).xcCounter = {};
    await mount(false);
    const link = host.querySelector<HTMLAnchorElement>('[data-counter-exit-link]');
    expect(link?.getAttribute('href')).toBe('tauri://localhost/exit.html');
  });

  it('U2: a plain browser has no exit link', async () => {
    await mount(true);
    expect(host.querySelector('[data-counter-exit-link]')).toBeNull();
  });

  it('U3: without the permission there is no panel and nothing is read', async () => {
    await mount(false);
    expect(host.querySelector('[data-counter-unlock]')).toBeNull();
    expect(calls).toEqual([]);
  });

  it("U4: opening the panel reads the branch's counters", async () => {
    await mount(true);
    expect(calls).toEqual([]);
    const details = await openPanel();
    expect(calls.map((c) => c.url)).toEqual([`${VIEW}?branch=MAIN`]);
    const options = [...details.querySelectorAll('option')].map((o) => o.value);
    expect(options).toEqual(['agent-1', 'agent-2']);
  });

  it('U5: issuing sends the chosen counter and shows the code', async () => {
    await mount(true);
    const details = await openPanel();
    const select = details.querySelector('select')!;
    select.value = 'agent-2';
    select.dispatchEvent(new Event('change'));
    details.querySelector('button')!.click();
    await settle();
    const issue = calls.filter((c) => c.url === ISSUE);
    expect(issue).toHaveLength(1);
    expect(issue[0].method).toBe('POST');
    expect(issue[0].body).toEqual({ agentId: 'agent-2' });
    expect(issue[0].key).toBeTruthy();
    expect(details.querySelector('[data-counter-unlock-code]')?.textContent).toContain('ABCDE-FGHJK');
  });

  it('U6: a refusal is worded in an alert and no code is shown', async () => {
    answerIssue = () =>
      json({ error: { code: 'roleNotAllowedHere', message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้' } }, 400);
    await mount(true);
    const details = await openPanel();
    details.querySelector('button')!.click();
    await settle();
    expect(details.querySelector('[role="alert"]')?.textContent).toBeTruthy();
    expect(details.querySelector('[data-counter-unlock-code]')).toBeNull();
  });
});
