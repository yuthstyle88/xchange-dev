import { For, Show, type JSX } from 'solid-js';
import type * as table from './i18n';
import type { RatePolicyData, ValueTierDto } from '../../../shared/api/generated/rate_policy';
import type { RatePolicySaveInput, RatePolicySaveOutput } from '../../../shared/api/generated/rate_policy_save';
import { wordedError } from '../../../shared/i18n/worded-error';

export interface TierFormRow {
  tierNo: 1 | 2 | 3 | 4;
  upperInclusiveThb: string;
  buyMarginPct: string;
  sellMarginPct: string;
}

export function emptyValueTiers(): TierFormRow[] {
  return ([1, 2, 3, 4] as const).map((tierNo) => ({
    tierNo,
    upperInclusiveThb: '',
    buyMarginPct: '',
    sellMarginPct: '',
  }));
}

function parsePositiveDecimalParts(raw: string): { intPart: string; fracPart: string } | null {
  const trimmed = raw.trim();
  const m = /^\+?(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!m) return null;
  const intPart = m[1]!.replace(/^0+/, '') || '0';
  const fracPart = (m[2] ?? '').replace(/0+$/, '');
  if (intPart === '0' && fracPart === '') return null;
  return { intPart, fracPart };
}

function isDecimalLessThan(
  a: { intPart: string; fracPart: string },
  b: { intPart: string; fracPart: string },
): boolean {
  if (a.intPart.length !== b.intPart.length) {
    return a.intPart.length < b.intPart.length;
  }
  if (a.intPart !== b.intPart) {
    return a.intPart < b.intPart;
  }
  const maxFrac = Math.max(a.fracPart.length, b.fracPart.length);
  const aFrac = a.fracPart.padEnd(maxFrac, '0');
  const bFrac = b.fracPart.padEnd(maxFrac, '0');
  return aFrac < bFrac;
}

export interface RatesPolicyCardProps {
  branch: () => string;
  busy: () => boolean;
  canSavePolicy: () => boolean;
  policyData: () => RatePolicyData | null;
  policyError: () => string | null;
  valueTiers: () => TierFormRow[];
  setValueTiers: (updater: TierFormRow[] | ((prev: TierFormRow[]) => TierFormRow[])) => void;
  valueTiersOn: () => boolean;
  setValueTiersOn: (val: boolean) => void;
  sharpMoveAlertPct: () => string;
  setSharpMoveAlertPct: (val: string) => void;
  adjustBuyPct: () => string;
  adjustSellPct: () => string;
  savePolicyWrite: {
    send(body: RatePolicySaveInput & { branch: string }): Promise<RatePolicySaveOutput>;
    reset(): void;
  };
  readGen: () => number;
  setSimulateResult: (val: null) => void;
  guarded: (fn: () => Promise<void>) => () => Promise<void>;
  toast: (msg: string) => void;
  t: (key: keyof typeof table.th) => string;
}

export function RatesPolicyCard(props: RatesPolicyCardProps): JSX.Element {
  const nonRisingTierNo = (): 1 | 2 | 3 | null => {
    if (!props.valueTiersOn()) return null;
    const rows = props.valueTiers();
    const u1Raw = rows[0]?.upperInclusiveThb ?? '';
    const u2Raw = rows[1]?.upperInclusiveThb ?? '';
    const u3Raw = rows[2]?.upperInclusiveThb ?? '';
    const u1 = parsePositiveDecimalParts(u1Raw);
    const u2 = parsePositiveDecimalParts(u2Raw);
    const u3 = parsePositiveDecimalParts(u3Raw);

    if (u1Raw.trim() !== '' && u1 === null) return 1;
    if (u2Raw.trim() !== '' && u2 === null) return 2;
    if (u1 !== null && u2 !== null && !isDecimalLessThan(u1, u2)) return 2;
    if (u3Raw.trim() !== '' && u3 === null) return 3;
    if (u2 !== null && u3 !== null && !isDecimalLessThan(u2, u3)) return 3;
    if (u1 !== null && u3 !== null && !isDecimalLessThan(u1, u3)) return 3;
    return null;
  };

  const areValueTiersValid = (): boolean => {
    if (!props.valueTiersOn()) return true;
    const rows = props.valueTiers();
    if (rows.length !== 4) return false;
    const u1 = parsePositiveDecimalParts(rows[0]!.upperInclusiveThb);
    const u2 = parsePositiveDecimalParts(rows[1]!.upperInclusiveThb);
    const u3 = parsePositiveDecimalParts(rows[2]!.upperInclusiveThb);
    if (!u1 || !u2 || !u3) return false;
    if (!isDecimalLessThan(u1, u2) || !isDecimalLessThan(u2, u3)) return false;
    if (rows[3]!.upperInclusiveThb !== '') return false;
    return rows.every((r) => r.buyMarginPct.trim() !== '' && r.sellMarginPct.trim() !== '');
  };

  const updateTierField = (
    tierNo: 1 | 2 | 3 | 4,
    field: 'upperInclusiveThb' | 'buyMarginPct' | 'sellMarginPct',
    value: string,
  ) => {
    props.setValueTiers((prev) =>
      prev.map((row) => (row.tierNo === tierNo ? { ...row, [field]: value } : row)),
    );
  };

  const handleSavePolicy = props.guarded(async () => {
    if (!props.canSavePolicy()) return;
    const policy = props.policyData()?.policy;
    if (!policy || Boolean(props.policyError())) return;
    if (!areValueTiersValid()) return;

    const branch = props.branch();
    const gen = props.readGen();
    const rows = props.valueTiers();
    const hasAnyTierInput = rows.some(
      (r) => r.upperInclusiveThb !== '' || r.buyMarginPct !== '' || r.sellMarginPct !== '',
    );
    const tiersPayload: ValueTierDto[] =
      !props.valueTiersOn() && !hasAnyTierInput
        ? []
        : rows.map((r) => ({
            tierNo: r.tierNo,
            upperInclusiveThb:
              r.tierNo === 4 ? null : r.upperInclusiveThb === '' ? null : r.upperInclusiveThb,
            buyMarginPct: r.buyMarginPct,
            sellMarginPct: r.sellMarginPct,
          }));

    const payload: RatePolicySaveInput & { branch: string } = {
      branch,
      buyAdjustPct: props.adjustBuyPct(),
      sellAdjustPct: props.adjustSellPct(),
      guards: policy.guards,
      maxReferenceAgeHours: policy.maxReferenceAgeHours,
      onStale: policy.onStale,
      staleLimitThb: policy.staleLimitThb,
      reason: policy.reason || props.t('savePolicy'),
      valueTiersOn: props.valueTiersOn(),
      valueTiers: tiersPayload,
      sharpMoveAlertPct: props.sharpMoveAlertPct() === '' ? null : props.sharpMoveAlertPct(),
    };

    try {
      await props.savePolicyWrite.send(payload);
      if (gen !== props.readGen()) return;
      props.setSimulateResult(null);
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  return (
    <div class="rates-section">
      <div class="section-header">
        <div class="title-row">
          <div>
            <div class="eyebrow">{props.t('policyEyebrow')}</div>
            <h2>{props.t('policyTitle')}</h2>
          </div>
          <div class="dialog-actions">
            <button
              type="button"
              data-action="save-policy"
              data-testid="save-policy"
              disabled={
                props.busy() ||
                !props.canSavePolicy() ||
                Boolean(props.policyError()) ||
                !props.policyData()?.policy ||
                !areValueTiersValid()
              }
              onClick={handleSavePolicy}
            >
              {props.t('savePolicy')}
            </button>
          </div>
        </div>
        <Show when={nonRisingTierNo() !== null}>
          <div
            class="form-error"
            role="alert"
            data-field-error={`tier-${nonRisingTierNo()}`}
          >
            {`${props.t('tierNonRisingPrefix')}${nonRisingTierNo()}${props.t('tierNonRisingSuffix')}`}
          </div>
        </Show>
      </div>

      <div class="adjust-all-bar" style={{ padding: '12px 24px', display: 'flex', gap: '24px', "align-items": 'center', "flex-wrap": 'wrap' }}>
        <label class="adjust-field-label" style={{ display: 'flex', gap: '8px', "align-items": 'center' }}>
          <input
            type="checkbox"
            name="valueTiersOn"
            data-field="valueTiersOn"
            data-testid="value-tiers-switch"
            checked={props.valueTiersOn()}
            disabled={!props.canSavePolicy()}
            onChange={(e) => {
              props.setValueTiersOn(e.currentTarget.checked);
            }}
          />
          <span>{props.t('valueTiersSwitchLabel')}</span>
        </label>

        <label class="adjust-field-label">
          {props.t('sharpMoveThresholdLabel')}
          <input
            type="text"
            name="sharpMoveAlertPct"
            data-field="sharpMoveAlertPct"
            data-testid="sharp-move-threshold"
            value={props.sharpMoveAlertPct()}
            disabled={!props.canSavePolicy()}
            onInput={(e) => {
              props.setSharpMoveAlertPct(e.currentTarget.value);
            }}
          />
        </label>
      </div>

      <div class="table-scroll">
        <table class="rates-table">
          <thead>
            <tr>
              <th>{props.t('headerTierNo')}</th>
              <th class="num">{props.t('headerTierUpperThb')}</th>
              <th class="num">{props.t('headerTierBuyMargin')}</th>
              <th class="num">{props.t('headerTierSellMargin')}</th>
            </tr>
          </thead>
          <tbody>
            <For each={[1, 2, 3, 4] as const}>
              {(tierNo) => {
                const row = () => props.valueTiers()[tierNo - 1]!;
                return (
                  <tr data-tier-no={tierNo}>
                    <td>{`${props.t('tierLabelPrefix')}${tierNo}`}</td>
                    <td class="num">
                      <Show
                        when={tierNo < 4}
                        fallback={<span>{props.t('tierAndAbove')}</span>}
                      >
                        <input
                          type="text"
                          name={`tier-${tierNo}-upperInclusiveThb`}
                          data-field="upperInclusiveThb"
                          data-tier-field="upper"
                          value={row().upperInclusiveThb}
                          disabled={!props.canSavePolicy()}
                          aria-invalid={nonRisingTierNo() === tierNo ? 'true' : undefined}
                          onInput={(e) =>
                            updateTierField(tierNo, 'upperInclusiveThb', e.currentTarget.value)
                          }
                        />
                        <Show when={nonRisingTierNo() === tierNo}>
                          <div class="field-error" data-error>
                            {`${props.t('tierNonRisingPrefix')}${tierNo}${props.t('tierNonRisingSuffix')}`}
                          </div>
                        </Show>
                      </Show>
                    </td>
                    <td class="num">
                      <input
                        type="text"
                        name={`tier-${tierNo}-buyMarginPct`}
                        data-field="buyMarginPct"
                        data-tier-field="buy-margin"
                        value={row().buyMarginPct}
                        disabled={!props.canSavePolicy()}
                        onInput={(e) =>
                          updateTierField(tierNo, 'buyMarginPct', e.currentTarget.value)
                        }
                      />
                    </td>
                    <td class="num">
                      <input
                        type="text"
                        name={`tier-${tierNo}-sellMarginPct`}
                        data-field="sellMarginPct"
                        data-tier-field="sell-margin"
                        value={row().sellMarginPct}
                        disabled={!props.canSavePolicy()}
                        onInput={(e) =>
                          updateTierField(tierNo, 'sellMarginPct', e.currentTarget.value)
                        }
                      />
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
