// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// F6 FE (#564 finding F6; server side PR 6042 / #6038, design docs/design/blueprint/user-set-settings.md
// "Every setting names who saved it, by name, from its own row"): #cash-policy names who set the limit by name.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests (the banner line `current`, th `ฉบับ {version} · ตั้งโดย {setBy} · {when}`, unchanged):
// - N1: `{setBy}` is the policy's `setByName`.
// - N2: `setByName` null ⇒ "—"; the raw id (`setBy`) is never shown anywhere on the screen.
// - N3: in English the same, through `Version {version} · set by {setBy} · {when}`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { makeCashTransferPolicyData, makeMe } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { menu } from './menu';

const SETTER_ID = 'u-exec-7f3a';
const CREATED = '2026-11-03T02:00:00Z';

// F55: the banner's time is pinned (was th-TH in the device's zone, a Buddhist year and seconds).
// CREATED 02:00Z is 09:00 in Bangkok.
const when = (iso: string) => {
  if (iso !== CREATED) throw new Error(`no pinned F55 text for ${iso}`);
  return '03/11/2026 09:00';
};

function policy(setByName: string | null) {
  return makeCashTransferPolicyData({
    version: 'v-0003',
    limits: { USD: '300.00' },
    valid: true,
    invalidEntries: [],
    setBy: SETTER_ID,
    setByName,
    createdAt: CREATED,
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const banner = () => root.querySelector('[data-section="cash-policy"] .stock-banner strong');

async function mount(answer: ReturnType<typeof policy>) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) =>
      String(input).includes('/api/v3/views/cash_transfer_policy')
        ? json(200, { changed: true, versions: {}, data: answer })
        : json(200, { version: 1, changed: true, data: null }),
    ),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await vi.waitFor(() => expect(text(banner())).toContain('v-0003'));
}

beforeEach(() => {
  sharedSession().updateMe(makeMe({ permissions: ['settingsRead'] }));
  setLang('th');
});

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('F6 #cash-policy names who set the transfer limit', () => {
  it('N1 the banner names the setter by name', async () => {
    await mount(policy('ผู้บริหาร ค'));
    expect(text(banner())).toBe(`ฉบับ v-0003 · ตั้งโดย ผู้บริหาร ค · ${when(CREATED)}`);
    expect(text(root)).not.toContain(SETTER_ID);
  });

  it('N2 a setter with no name reads "—", never the id', async () => {
    await mount(policy(null));
    expect(text(banner())).toBe(`ฉบับ v-0003 · ตั้งโดย — · ${when(CREATED)}`);
    expect(text(root)).not.toContain(SETTER_ID);
  });

  it('N3 in English', async () => {
    setLang('en');
    await mount(policy('ผู้บริหาร ค'));
    expect(text(banner())).toBe(`Version v-0003 · set by ผู้บริหาร ค · ${when(CREATED)}`);
  });
});
