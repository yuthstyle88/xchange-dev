// F4 FE (#564 finding F4; server side PR 6042 / #6038, design docs/design/blueprint/user-set-settings.md
// "Every setting names who saved it, by name, from its own row"): the drawer low-level card names its own setter.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - L1: `drawerLowLevelsUpdatedAt` set ⇒ the card shows one `[data-testid="levels-set-by"]` line:
//   th `ตั้งโดย {by} · {at}`, `{by}` = `drawerLowLevelsUpdatedByName`, `{at}` = the time as the page's banner writes it
//   (`toLocaleString(locale(), { timeZone: 'Asia/Bangkok' })`). It never shows the limit's setter (`updatedByName`).
// - L2: `drawerLowLevelsUpdatedByName` null ⇒ `{by}` is "—"; the raw id (`drawerLowLevelsUpdatedBy`) is never shown.
// - L3: `drawerLowLevelsUpdatedAt` null (no level saved) ⇒ no setter line.
// - L4: the line follows the data the card is given (the save's answer replaces it).
// - L5: on the #cash-limits page the banner keeps naming the limit's setter, and the card names the levels' setter.
// - L6: in English, `Set by {by} · {at}`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { makeCashLimitsData, makeDrawerLowLevel, makeMe } from '../../../shared/testing/fixtures';
import { locale, setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { Toast } from '../parts/toast';
import { CashLimits } from './CashLimits';
import { DrawerLowLevels } from './DrawerLowLevels';

const LIMIT_SETTER = 'การเงิน ข';
const LEVELS_SETTER = 'เจ้าของ ก';
const LEVELS_AT = '2026-11-03T03:15:00Z';
const LIMIT_AT = '2026-11-02T09:00:00Z';

const at = (iso: string) => new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });

function data(over: Partial<Parameters<typeof makeCashLimitsData>[0]> = {}) {
  return makeCashLimitsData({
    branch: 'MAIN',
    countDifferenceLimitThb: '50.00',
    updatedBy: 'u-finance',
    updatedByName: LIMIT_SETTER,
    updatedAt: LIMIT_AT,
    drawerLowLevels: [makeDrawerLowLevel({ currency: 'THB', lowLevel: '6000.00' })],
    drawerLowLevelsUpdatedBy: 'u-owner',
    drawerLowLevelsUpdatedByName: LEVELS_SETTER,
    drawerLowLevelsUpdatedAt: LEVELS_AT,
    ...over,
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const setterLine = () => root.querySelector('[data-section="drawer-low-levels"] [data-testid="levels-set-by"]');

async function mountCard(initial: ReturnType<typeof data>) {
  const [d, setD] = createSignal<ReturnType<typeof data> | null>(initial);
  const [branch] = createSignal('MAIN');
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <DrawerLowLevels data={d} branch={branch} onSaved={() => {}} />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await vi.waitFor(() => expect(root.querySelector('[data-section="drawer-low-levels"]')).not.toBeNull());
  return setD;
}

beforeEach(() => {
  sharedSession().updateMe(makeMe({ permissions: ['CashLimitsManage'] }));
  vi.stubGlobal('fetch', vi.fn(async () => json(404, { error: { code: 'notFound', message: 'nf' } })));
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('F4 the drawer low-level card names its own setter', () => {
  it("L1 the levels' setter and time, not the limit's", async () => {
    await mountCard(data());
    expect(root.querySelectorAll('[data-testid="levels-set-by"]')).toHaveLength(1);
    expect(text(setterLine())).toBe(`ตั้งโดย ${LEVELS_SETTER} · ${at(LEVELS_AT)}`);
    expect(text(root)).not.toContain(LIMIT_SETTER);
  });

  it('L2 a setter with no name reads "—", never the id', async () => {
    await mountCard(data({ drawerLowLevelsUpdatedByName: null }));
    expect(text(setterLine())).toBe(`ตั้งโดย — · ${at(LEVELS_AT)}`);
    expect(text(root)).not.toContain('u-owner');
  });

  it('L3 no level saved: no setter line', async () => {
    await mountCard(
      data({
        drawerLowLevels: [],
        drawerLowLevelsUpdatedBy: null,
        drawerLowLevelsUpdatedByName: null,
        drawerLowLevelsUpdatedAt: null,
      }),
    );
    expect(setterLine()).toBeNull();
  });

  it('L4 the line follows the data the card is given', async () => {
    const setD = await mountCard(data({ drawerLowLevelsUpdatedByName: 'คนก่อน' }));
    expect(text(setterLine())).toContain('คนก่อน');
    setD(data({ drawerLowLevelsUpdatedAt: '2026-11-03T04:00:00Z' }));
    await vi.waitFor(() => expect(text(setterLine())).toBe(`ตั้งโดย ${LEVELS_SETTER} · ${at('2026-11-03T04:00:00Z')}`));
  });

  it("L5 on the #cash-limits page the banner names the limit's setter and the card the levels'", async () => {
    const answer = data();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) =>
        String(input).includes('/api/v3/views/cash_limits')
          ? json(200, { version: 1, changed: true, data: answer })
          : json(404, { error: { code: 'notFound', message: 'nf' } }),
      ),
    );
    root = document.createElement('div');
    document.body.appendChild(root);
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
    dispose = render(() => <CashLimits branch={branch} period={period} shown={() => true} />, root);
    await vi.waitFor(() => expect(setterLine()).not.toBeNull());
    const card = root.querySelector('[data-section="drawer-low-levels"]');
    const outside = text(root).replace(text(card), '');
    expect(outside, 'the banner keeps the limit setter').toContain(LIMIT_SETTER);
    expect(outside).not.toContain(LEVELS_SETTER);
    expect(text(setterLine())).toBe(`ตั้งโดย ${LEVELS_SETTER} · ${at(LEVELS_AT)}`);
  });

  it('L6 in English: Set by {by} · {at}', async () => {
    setLang('en');
    await mountCard(data());
    expect(text(setterLine())).toBe(`Set by ${LEVELS_SETTER} · ${at(LEVELS_AT)}`);
  });
});
