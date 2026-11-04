import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../i18n';
import {
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../testing/fixtures';
import { CountForm } from '../../../screens/staff/shift/CountForm';
import { CurrencyCount } from './CurrencyCount';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'ALL' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '100.0000', groupCode: 'ALL' }),
  makeDenominationRow({ id: 31, currency: 'EUR', faceValue: '50.0000', groupCode: 'ALL' }),
];

describe('BC-10 CurrencyCount countable prop', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  function typeSearch(value: string) {
    const input = host.querySelector<HTMLInputElement>('[data-testid="currency-search"]')!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  const options = () =>
    [...host.querySelectorAll<HTMLElement>('[data-testid="currency-option"]')].map((el) =>
      el.getAttribute('data-currency'),
    );

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('en');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('with countable: search lists only those codes in countable that also have notes', () => {
    dispose = render(
      () => (
        <CurrencyCount
          denominations={DENOMS}
          countable={['THB', 'USD', 'KRW']}
          locked={['THB']}
          quantities={{}}
          setQuantity={() => {}}
          lineErrors={{}}
        />
      ),
      host,
    );

    typeSearch('usd');
    expect(options()).toEqual(['USD']);

    typeSearch('eur');
    expect(options()).toEqual([]);
    expect(host.querySelector('[data-testid="currency-notfound"]')).not.toBeNull();

    typeSearch('krw');
    expect(options()).toEqual([]);
    expect(host.querySelector('[data-testid="currency-notfound"]')).not.toBeNull();
  });

  it('without countable: lists every currency with notes as today', () => {
    dispose = render(
      () => (
        <CurrencyCount
          denominations={DENOMS}
          locked={['THB']}
          quantities={{}}
          setQuantity={() => {}}
          lineErrors={{}}
        />
      ),
      host,
    );

    typeSearch('usd');
    expect(options()).toEqual(['USD']);

    typeSearch('eur');
    expect(options()).toEqual(['EUR']);
  });

  it('CountForm reads branch_currencies and passes the codes as countable', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.startsWith('/api/v3/views/branch_currencies')) {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                rows: [
                  { code: 'THB', group: 'ALL', display: 'HIDE', sortOrder: 1, held: false },
                  { code: 'USD', group: 'ALL', display: 'SHOW', sortOrder: 2, held: true },
                  { code: 'KRW', group: 'KRWS', display: 'ASK', sortOrder: 21, held: false },
                ],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response('{}', { status: 404 });
      }),
    );

    const [busy, setBusy] = createSignal(false);
    const [denoms] = createSignal(DENOMS);
    dispose = render(
      () => (
        <CountForm
          ctx={{
            me: makeStaffMeView({ branchId: 'MAIN', permissions: ['ShiftOperate'] }),
            floor: makeStaffFloorView({ phase: 'OPEN', myClockIn: '2026-11-04T01:00:00Z', myShift: null }),
            refresh: async () => {},
          }}
          kind="OPEN"
          denominations={denoms}
          busy={busy}
          setBusy={setBusy}
          onSaved={async () => {}}
          onCancel={() => {}}
          onError={() => {}}
        />
      ),
      host,
    );

    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));

    typeSearch('usd');
    expect(options()).toEqual(['USD']);

    typeSearch('eur');
    expect(options()).toEqual([]);
    expect(host.querySelector('[data-testid="currency-notfound"]')).not.toBeNull();
  });
});
