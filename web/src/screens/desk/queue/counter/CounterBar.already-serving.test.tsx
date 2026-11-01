// FE-T1a #5851 (design docs/design/fe-triage-1101.md §3 W3; triage T7(a) on #564). Written before the implementation by
// the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
//
// [required] by these tests:
// - A `409 alreadyServing` refusal of call next shows `./i18n.ts` key `error.alreadyServing` (th and en, W3 text),
//   never the shared `error.serverError` wording, and the board is not refreshed.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { en as sharedEn } from '../../../../shared/i18n/en';
import { CounterBar } from './CounterBar';
import { en, th } from './i18n';

const TELLER = { userId: 'u-me', permissions: ['QueueOperate'] };
const W3_TH = 'เคาน์เตอร์นี้กำลังให้บริการคิวอื่นอยู่ กรุณาเสร็จสิ้นคิวปัจจุบันก่อน';
const W3_EN = 'This counter is already serving a ticket. Finish it first.';

function fakeStore() {
  const [state] = createSignal<any>({ waiting: [], called: [], myCounter: { status: 'ACTIVE' } });
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
  return {
    issueWalkIn: vi.fn(),
    callNext: vi.fn(async () => {
      throw new ApiError({ status: 409, code: 'alreadyServing', message: 'alreadyServing' });
    }),
    ticket: vi.fn(),
    cancel: vi.fn(),
    counter: vi.fn(),
    queue: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});
function mount(el: () => JSX.Element) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}
const button = (n: string) => host.querySelector<HTMLButtonElement>(`button[name=${n}]`);
async function flush() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe('CounterBar: alreadyServing has its own wording (FE-T1a W3)', () => {
  it('T1a-4a the i18n key carries the W3 text in th and en', () => {
    expect((th as Record<string, string>)['error.alreadyServing']).toBe(W3_TH);
    expect((en as Record<string, string>)['error.alreadyServing']).toBe(W3_EN);
  });

  it('T1a-4 th: a 409 alreadyServing on call next shows the W3 text, not error.serverError', async () => {
    const store = fakeStore();
    mount(() => <CounterBar store={store} me={TELLER} commands={fakeCommands()} />);
    button('callNext')!.click();
    await flush();
    const text = host.querySelector('[role=alert]')?.textContent ?? '';
    expect(text).toContain(W3_TH);
    expect(text).not.toContain(sharedTh['error.serverError']);
    expect(store.refresh).not.toHaveBeenCalled();
  });

  it('T1a-4b en: a 409 alreadyServing on call next shows the W3 text, not error.serverError', async () => {
    setLang('en');
    mount(() => <CounterBar store={fakeStore()} me={TELLER} commands={fakeCommands()} />);
    button('callNext')!.click();
    await flush();
    const text = host.querySelector('[role=alert]')?.textContent ?? '';
    expect(text).toContain(W3_EN);
    expect(text).not.toContain(sharedEn['error.serverError']);
  });
});
