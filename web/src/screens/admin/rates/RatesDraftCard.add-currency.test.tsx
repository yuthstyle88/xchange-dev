// DC-1 (xchange#564 D-a gap) & BC-6 (xchange#6085): an editable rate draft adds a currency from rates_draftable.
// Design: docs/design/board-currencies.md § BC-6.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeRateDraftDetailData,
  makeRateDraftDetailItemDto,
  makeRateDraftLineDto,
  makeDraftableCurrency,
  makeRatesDraftableData,
  makeRatesReferenceData,
  makeRatesReferenceRow,
} from '../../../shared/testing/fixtures';
import type { ApiClient } from '../../../shared/api/client';
import { RatesDraftCard } from './RatesDraftCard';
import * as ratesTable from './i18n';

const WORDING = {
  th: { addCurrency: 'เพิ่มสกุลเงิน', noCurrencyToAdd: 'ไม่มีสกุลเงินจากแหล่งอ้างอิงให้เพิ่ม' },
  en: { addCurrency: 'Add currency', noCurrencyToAdd: 'No reference currency left to add' },
} as const;

type Detail = ReturnType<typeof makeRateDraftDetailData>;
type Reference = ReturnType<typeof makeRatesReferenceData>;
type Draftable = ReturnType<typeof makeRatesDraftableData>;

function detail(status: string): Detail {
  return makeRateDraftDetailData({
    draft: makeRateDraftDetailItemDto({ id: 'd-1', branchId: 'MAIN', status, version: 3 }),
    lines: [makeRateDraftLineDto({ currency: 'USD', buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null })],
  });
}

const DRAFTABLE = makeRatesDraftableData({
  rows: [
    makeDraftableCurrency({ code: 'USD', group: 'ALL', unitSize: '1', hasReference: true }),
    makeDraftableCurrency({ code: 'EUR', group: 'ALL', unitSize: '1', hasReference: true }),
    makeDraftableCurrency({ code: 'JPY', group: 'ALL', unitSize: '100', hasReference: true }),
    makeDraftableCurrency({ code: 'CNY', group: 'ALL', unitSize: '1', hasReference: true }),
  ],
});

const REFERENCE = makeRatesReferenceData({
  rows: ['USD', 'EUR', 'JPY', 'CNY', 'KRW'].map((code) =>
    makeRatesReferenceRow({ code, unit: code === 'JPY' ? 100 : 1 }),
  ),
});

describe('BC-6: an editable draft adds a currency from rates_draftable', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let puts: { path: string; body: { lines?: { currency: string; buyMode: string; sellMode: string }[] } }[];

  beforeEach(() => {
    setLang('th');
    puts = [];
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  function mount(status: string, draftableData: Draftable | null, refData: Reference | null = REFERENCE): void {
    const [draftDetail, setDraftDetail] = createSignal<Detail | null>(detail(status));
    dispose = render(
      () => (
        <RatesDraftCard
          branch={() => 'MAIN'}
          busy={() => false}
          draftDetail={draftDetail}
          setDraftDetail={setDraftDetail}
          simulateResult={() => null}
          setSimulateResult={() => {}}
          draftsError={() => null}
          policyError={() => null}
          policyData={() => null}
          pubData={() => null}
          draftableData={() => draftableData}
          refData={() => refData}
          adjustBuyPct={() => '0'}
          setAdjustBuyPct={() => {}}
          adjustSellPct={() => '0'}
          setAdjustSellPct={() => {}}
          client={{ view: vi.fn(async () => ({ version: 1, changed: true, data: null })) } as unknown as ApiClient}
          writeFor={<B, R>(_id: string, method: string, path: string) => ({
            async send(body: B): Promise<R> {
              if (method === 'PUT') puts.push({ path, body: body as (typeof puts)[number]['body'] });
              return { id: 'd-1', version: 4, status: 'DRAFT' } as R;
            },
            reset() {},
          })}
          readDrafts={async () => {}}
          readPublished={async () => {}}
          readGen={() => 1}
          guarded={(fn) => fn}
          toast={() => {}}
          t={(k) => ratesTable.th[k]}
        />
      ),
      host,
    );
  }

  const picker = () => host.querySelector<HTMLSelectElement>('select[name="addCurrency"]');
  const addButton = () => host.querySelector<HTMLButtonElement>('button[data-action="add-currency"]');
  const options = () => Array.from(picker()?.options ?? []).map((o) => o.value);
  const settle = async () => {
    for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
  };

  it('L1: the two new keys exist in th and en with the ticket wording', () => {
    for (const lang of ['th', 'en'] as const) {
      for (const key of Object.keys(WORDING.th) as (keyof typeof WORDING.th)[]) {
        expect((ratesTable[lang] as Record<string, string>)[key], `${lang}.${key}`).toBe(WORDING[lang][key]);
      }
    }
  });

  it('A1: the picker offers the draftable codes not in the draft, A to Z', async () => {
    mount('DRAFT', DRAFTABLE);
    await settle();
    expect(picker(), 'add-currency picker').not.toBeNull();
    expect(options()).toEqual(['CNY', 'EUR', 'JPY']);
    expect(addButton()?.textContent?.trim()).toBe(WORDING.th.addCurrency);
  });

  it('A2: adding EUR shows an EUR draft line and drops EUR from the picker', async () => {
    mount('DRAFT', DRAFTABLE);
    await settle();
    picker()!.value = 'EUR';
    picker()!.dispatchEvent(new Event('change', { bubbles: true }));
    addButton()!.click();
    await settle();
    expect(host.querySelector('[data-draft-line="EUR"]'), 'EUR line').not.toBeNull();
    expect(host.querySelector('[data-draft-line="USD"]'), 'USD line kept').not.toBeNull();
    expect(options()).toEqual(['CNY', 'JPY']);
  });

  it('A3: saving sends the added currency as an AUTO/AUTO line after the existing ones', async () => {
    mount('DRAFT', DRAFTABLE);
    await settle();
    picker()!.value = 'JPY';
    picker()!.dispatchEvent(new Event('change', { bubbles: true }));
    addButton()!.click();
    await settle();
    host.querySelector<HTMLButtonElement>('button.btn-save-lines')!.click();
    await settle();
    expect(puts).toHaveLength(1);
    expect(puts[0]!.path).toBe('/api/rate-drafts/d-1');
    expect(puts[0]!.body.lines?.map((l) => l.currency)).toEqual(['USD', 'JPY']);
    expect(puts[0]!.body.lines?.[1]).toEqual({
      currency: 'JPY',
      buyMode: 'AUTO',
      sellMode: 'AUTO',
      manualBuy: null,
      manualSell: null,
      reason: null,
    });
  });

  it('A4: when rates_draftable is unreadable, nothing can be added and the existing hint is shown', async () => {
    mount('DRAFT', null);
    await settle();
    expect(picker()).toBeNull();
    expect(addButton()).toBeNull();
    const hint = host.querySelector('[data-hint="no-currency-to-add"]');
    expect(hint, 'hint').not.toBeNull();
    expect(hint!.textContent?.trim()).toBe(WORDING.th.noCurrencyToAdd);
  });

  it('A5: a draft that is not editable (PENDING_APPROVAL) shows neither the picker nor the hint', async () => {
    mount('PENDING_APPROVAL', DRAFTABLE);
    await settle();
    expect(picker()).toBeNull();
    expect(addButton()).toBeNull();
    expect(host.querySelector('[data-hint="no-currency-to-add"]')).toBeNull();
  });

  it('A6: adding TRY (hasReference: false) starts the line as MANUAL/MANUAL', async () => {
    mount('DRAFT', {
      rows: [
        ...DRAFTABLE.rows,
        { code: 'TRY', group: 'ALL', unitSize: '1', hasReference: false },
      ],
    });
    await settle();
    picker()!.value = 'TRY';
    picker()!.dispatchEvent(new Event('change', { bubbles: true }));
    addButton()!.click();
    await settle();
    const line = host.querySelector<HTMLElement>('[data-draft-line="TRY"]');
    expect(line, 'TRY line').not.toBeNull();
    expect(line!.querySelector<HTMLSelectElement>('select[name="buyMode"]')?.value).toBe('MANUAL');
    expect(line!.querySelector<HTMLSelectElement>('select[name="sellMode"]')?.value).toBe('MANUAL');
    expect(line!.querySelector<HTMLInputElement>('input[name="manualBuy"]')).not.toBeNull();
    expect(line!.querySelector<HTMLInputElement>('input[name="manualSell"]')).not.toBeNull();
  });

  it('A7: adding KRW starts the line as AUTO/AUTO and formats per 1000 from rates_draftable', async () => {
    mount('DRAFT', {
      rows: [
        ...DRAFTABLE.rows,
        { code: 'KRW', group: 'ALL', unitSize: '1000', hasReference: true },
      ],
    });
    await settle();
    picker()!.value = 'KRW';
    picker()!.dispatchEvent(new Event('change', { bubbles: true }));
    addButton()!.click();
    await settle();
    const line = host.querySelector<HTMLElement>('[data-draft-line="KRW"]');
    expect(line, 'KRW line').not.toBeNull();
    const buyMode = line!.querySelector<HTMLSelectElement>('select[name="buyMode"]')!;
    const sellMode = line!.querySelector<HTMLSelectElement>('select[name="sellMode"]')!;
    expect(buyMode.value).toBe('AUTO');
    expect(sellMode.value).toBe('AUTO');

    buyMode.value = 'MANUAL';
    buyMode.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    const manualBuy = line!.querySelector<HTMLInputElement>('input[name="manualBuy"]')!;
    manualBuy.value = '25.1';
    manualBuy.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(line!.textContent).toContain('25.10 / 1000 KRW');
  });
});
