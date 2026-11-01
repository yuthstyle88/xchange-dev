// DC-2 (xchange#564 D-a gap): the policy card lists a guard row for every currency in the open draft, so a currency
// that was never published can get its guard before the draft is submitted. Up-front tests by the FE sub-controller
// claude/01TxaRJG. Do not edit, delete or weaken. Design: docs/design/rates-draftable-currencies.md §2-§3.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedRatesData,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
} from '../../../shared/testing/fixtures';
import { RatesPolicyCard } from './RatesPolicyCard';
import * as ratesTable from './i18n';

vi.mock('./RatesFormulaOverride', () => ({ RatesFormulaOverride: () => null }));

const PUBLISHED = makePublishedRatesData({ lines: [makePublishedLineDto({ currency: 'USD' })] });
const POLICY = makeRatePolicyData({
  policy: makeRatePolicyVersionDto({ guards: { GBP: { buy: { min_pct: '-1', max_pct: '1' }, sell: { min_pct: '-1', max_pct: '1' } } } }),
});

describe('DC-2: the guard list includes the open draft currencies', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  function mount(draftCurrencies: string[] | undefined): void {
    const [valueTiers, setValueTiers] = createSignal(
      ([1, 2, 3, 4] as const).map((tierNo) => ({ tierNo, upperInclusiveThb: '', buyMarginPct: '', sellMarginPct: '' })),
    );
    const [valueTiersOn, setValueTiersOn] = createSignal(false);
    const [sharp, setSharp] = createSignal('');
    dispose = render(
      () => (
        <RatesPolicyCard
          branch={() => 'MAIN'}
          busy={() => false}
          canSavePolicy={() => true}
          policyData={() => POLICY}
          policyError={() => null}
          pubData={() => PUBLISHED}
          draftCurrencies={draftCurrencies === undefined ? undefined : () => draftCurrencies}
          valueTiers={valueTiers}
          setValueTiers={setValueTiers}
          valueTiersOn={valueTiersOn}
          setValueTiersOn={setValueTiersOn}
          sharpMoveAlertPct={sharp}
          setSharpMoveAlertPct={setSharp}
          adjustBuyPct={() => '0.0000'}
          adjustSellPct={() => '0.0000'}
          savePolicyWrite={{
            async send() {
              return { id: 'pol-1', version: 1, notChecked: [] };
            },
            reset() {},
          }}
          readGen={() => 1}
          setSimulateResult={() => {}}
          guarded={(fn) => fn}
          toast={() => {}}
          t={(k) => ratesTable.th[k]}
        />
      ),
      host,
    );
  }

  const guardRows = () => Array.from(host.querySelectorAll('[data-guard-line]')).map((r) => r.getAttribute('data-guard-line'));

  it('G1: published, then stored guards, then draft currencies not already listed', async () => {
    mount(['USD', 'EUR', 'JPY']);
    await Promise.resolve();
    expect(guardRows()).toEqual(['USD', 'GBP', 'EUR', 'JPY']);
  });

  it('G2: no draft currencies prop keeps today\'s list (published, then stored)', async () => {
    mount(undefined);
    await Promise.resolve();
    expect(guardRows()).toEqual(['USD', 'GBP']);
  });

  it('G3: a draft currency that is already published or stored is listed once', async () => {
    mount(['GBP', 'USD']);
    await Promise.resolve();
    expect(guardRows()).toEqual(['USD', 'GBP']);
  });
});
