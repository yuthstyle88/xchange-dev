// #5785 up-front tests (xchange #564, TEST2 survey): a branch whose `rate_policy` read succeeded with `policy: null` can save
// its first policy from the policy card. Written before the implementation by the ticket author claude/01TxaRJG. Do not
// edit, delete or weaken; add tests in a file of your own.
//
// Rates.tsx states the card sees (Rates.tsx:63-64, :193-230, :265):
//   pending   policyData() === null, policyError() === null
//   failed    policyData() === null, policyError() !== null   (#1687: never save made-up values over a policy we could not read)
//   no policy policyData() = { policy: null }, policyError() === null   (api rate_policy.rs:128-131; rate_policy_save.rs:324-335 writes version 1)
//   policy    policyData() = { policy: {...} }, policyError() === null
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  type RatePolicyData,
  type RatePolicySaveInput,
  type RatePolicySaveOutput,
} from '../../../shared/testing/fixtures';
import { RatesPolicyCard } from './RatesPolicyCard';
import * as ratesTable from './i18n';

// The formula override block reads its own view; it is not what these rows decide.
vi.mock('./RatesFormulaOverride', () => ({ RatesFormulaOverride: () => null }));

describe('P1: the policy card saves a branch first policy, and still refuses when the read failed', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: (RatePolicySaveInput & { branch: string })[];

  beforeEach(() => {
    setLang('th');
    writes = [];
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  function mount(policyData: RatePolicyData | null, policyError: string | null, branch = 'TEST2'): void {
    const [valueTiers, setValueTiers] = createSignal(
      ([1, 2, 3, 4] as const).map((tierNo) => ({ tierNo, upperInclusiveThb: '', buyMarginPct: '', sellMarginPct: '' })),
    );
    const [valueTiersOn, setValueTiersOn] = createSignal(false);
    const [sharp, setSharp] = createSignal('');
    dispose = render(
      () => (
        <RatesPolicyCard
          branch={() => branch}
          busy={() => false}
          canSavePolicy={() => true}
          policyData={() => policyData}
          policyError={() => policyError}
          valueTiers={valueTiers}
          setValueTiers={setValueTiers}
          valueTiersOn={valueTiersOn}
          setValueTiersOn={setValueTiersOn}
          sharpMoveAlertPct={sharp}
          setSharpMoveAlertPct={setSharp}
          adjustBuyPct={() => '0.0000'}
          adjustSellPct={() => '0.0000'}
          savePolicyWrite={{
            async send(body): Promise<RatePolicySaveOutput> {
              writes.push(body);
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

  const saveBtn = () => host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]')!;
  const ageInput = () => host.querySelector<HTMLInputElement>('input[name="maxReferenceAgeHours"]')!;

  async function typeAge(v: string): Promise<void> {
    ageInput().value = v;
    ageInput().dispatchEvent(new InputEvent('input', { bubbles: true }));
    await Promise.resolve();
  }

  async function clickSave(): Promise<void> {
    saveBtn().click();
    await new Promise((r) => setTimeout(r, 20));
  }

  it('row 1: read OK with policy null -> save is enabled and posts version 1 with the form defaults (RatesPolicyCard.tsx:275-283)', async () => {
    mount(makeRatePolicyData({ policy: null }), null);
    await vi.waitFor(() => expect(saveBtn()).not.toBeNull());

    expect(saveBtn().disabled, 'save-policy must be enabled for a branch with no policy yet').toBe(false);

    // The frame rule still applies: an empty max reference age is refused with its field error, nothing posted.
    await clickSave();
    expect(writes).toEqual([]);
    expect(host.querySelector('[data-field-error="maxReferenceAgeHours"]')).not.toBeNull();

    await typeAge('24');
    await clickSave();
    expect(writes).toHaveLength(1);
    expect(writes[0]).toEqual({
      branch: 'TEST2',
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: {},
      maxReferenceAgeHours: 24,
      onStale: 'STOP',
      staleLimitThb: null,
      reason: ratesTable.th.savePolicy,
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
  });

  it('row 2a: read failed -> save stays disabled and nothing is posted (#1687)', async () => {
    mount(null, 'could not read the policy');
    await vi.waitFor(() => expect(saveBtn()).not.toBeNull());
    await typeAge('24');

    expect(saveBtn().disabled).toBe(true);
    await clickSave();
    expect(writes).toEqual([]);
  });

  it('row 2b: read still pending -> save stays disabled and nothing is posted', async () => {
    mount(null, null);
    await vi.waitFor(() => expect(saveBtn()).not.toBeNull());
    await typeAge('24');

    expect(saveBtn().disabled).toBe(true);
    await clickSave();
    expect(writes).toEqual([]);
  });

  it('row 3: read OK with a policy -> unchanged: enabled, posts the loaded frame and the loaded reason', async () => {
    mount(
      makeRatePolicyData({
        policy: makeRatePolicyVersionDto({
          id: 'pol-3',
          branchId: 'MAIN',
          version: 3,
          buyAdjustPct: '0.0000',
          sellAdjustPct: '0.0000',
          guards: {},
          maxReferenceAgeHours: 12,
          onStale: 'STOP',
          staleLimitThb: null,
          reason: 'Loaded reason',
          valueTiersOn: false,
          valueTiers: [],
          sharpMoveAlertPct: null,
        }),
      }),
      null,
      'MAIN',
    );
    await vi.waitFor(() => expect(ageInput().value).toBe('12'));

    expect(saveBtn().disabled).toBe(false);
    await clickSave();
    expect(writes).toHaveLength(1);
    expect(writes[0]).toMatchObject({ branch: 'MAIN', maxReferenceAgeHours: 12, onStale: 'STOP', reason: 'Loaded reason' });
  });
});
