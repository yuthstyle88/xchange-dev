// §9 `Pricing` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §6, §9). Written before the implementation
// by the sub-controller claude/dell-claude-1. Do not edit, delete or weaken these tests; add your own in another file.
//
// [required] by these tests:
// - `Pricing(props: AdminMenuProps)` reads the view `pricing_board` once on mount with `{ branch: props.branch() }`,
//   through `new ApiClient()` (global fetch), and passes the filtered rows to `PricingGrid` (`rows` prop), in the
//   view's order.
// - `select[name=filter]` values `all` · `toApprove` · `review` · `current`; a non-`all` filter keeps the rows whose
//   status is in `FILTER_STATUSES[filter]` (pricing-board.types.ts).
// - `input[name=search]` keeps the rows whose currency code contains the text, case-insensitive; it combines with the
//   filter.
// - `button[name=reload]` reads the view once more and shows the new answer's rows.
// `PricingGrid` is replaced by a probe here, so this file depends on no other part.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { For } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import type { PricingBoardData } from './pricing-board.types';

vi.mock('./PricingGrid', () => ({
  PricingGrid: (p: { rows: readonly { currency: string }[] }) => (
    <ul data-probe="grid">
      <For each={p.rows}>{(r) => <li data-currency={r.currency} />}</For>
    </ul>
  ),
}));

import { Pricing } from './Pricing';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

async function mount(boards?: PricingBoardData[]) {
  const server = boardFetch(boards);
  vi.stubGlobal('fetch', vi.fn(server.fn));
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <Pricing branch={() => 'MAIN'} period={() => ({ from: '', to: '' })} shown={() => true} />,
    host,
  );
  await flush();
  return server;
}

const shown = () => [...host.querySelectorAll('[data-probe=grid] li')].map((li) => li.getAttribute('data-currency'));
function change(sel: string, value: string, ev = 'change') {
  const el = host.querySelector<HTMLInputElement | HTMLSelectElement>(sel);
  expect(el, `${sel} exists`).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event(ev, { bubbles: true }));
}

describe('Pricing', () => {
  it('P1 reads pricing_board once with the branch and lists every row in the view order', async () => {
    const server = await mount();
    const reads = server.calls.filter((c) => c.path === '/api/v3/views/pricing_board');
    expect(reads).toHaveLength(1);
    expect(shown()).toEqual(BOARD.rows.map((r) => r.currency));
  });

  it('P2 "to approve" keeps Tiers, Pending and Unpriced rows', async () => {
    await mount();
    change('select[name=filter]', 'toApprove');
    await flush();
    expect(shown()).toEqual(['USD', 'EUR', 'JPY', 'GBP', 'AUD']);
  });

  it('P3 "review" keeps Review rows and "current" keeps Current rows', async () => {
    await mount();
    change('select[name=filter]', 'review');
    await flush();
    expect(shown()).toEqual(['SGD']);
    change('select[name=filter]', 'current');
    await flush();
    expect(shown()).toEqual(['CNY']);
  });

  it('P4 search matches the currency code case-insensitively and combines with the filter', async () => {
    await mount();
    change('input[name=search]', 'us', 'input');
    await flush();
    expect(shown()).toEqual(['USD', 'AUD']);
    change('select[name=filter]', 'current');
    await flush();
    expect(shown()).toEqual([]);
  });

  it('P5 reload reads the view again and shows the new rows', async () => {
    const second: PricingBoardData = { ...BOARD, rows: BOARD.rows.slice(0, 2) };
    const server = await mount([BOARD, second]);
    const reload = host.querySelector<HTMLButtonElement>('button[name=reload]');
    expect(reload, 'button[name=reload] exists').not.toBeNull();
    reload!.click();
    await flush();
    expect(server.calls.filter((c) => c.path === '/api/v3/views/pricing_board')).toHaveLength(2);
    expect(shown()).toEqual(['USD', 'EUR']);
  });
});
