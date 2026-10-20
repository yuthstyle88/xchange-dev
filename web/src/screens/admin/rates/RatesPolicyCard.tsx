import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import type * as table from './i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RatePolicyData, ValueTierDto } from '../../../shared/api/generated/rate_policy';
import type { RatePolicySaveInput, RatePolicySaveOutput } from '../../../shared/api/generated/rate_policy_save';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Field } from '../../../shared/ui/Field';
import './rates-policy.css';

export interface TierFormRow {
  tierNo: 1 | 2 | 3 | 4;
  upperInclusiveThb: string;
  buyMarginPct: string;
  sellMarginPct: string;
}

export interface GuardFormRow {
  currency: string;
  buyMin: string;
  buyMax: string;
  sellMin: string;
  sellMax: string;
}

export interface FrameFormState {
  guards: GuardFormRow[];
  maxReferenceAgeHours: string;
  onStale: string;
  staleLimitThb: string;
}

export type FrameErrorField = 'guards' | 'maxReferenceAgeHours' | 'staleLimitThb';

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

function parseSignedScale4(raw: string): bigint | null {
  const m = /^([+-])?(\d+)(?:\.(\d{1,4}))?$/.exec(raw.trim());
  if (!m) return null;
  const sign = m[1] === '-' ? -1n : 1n;
  const scaled = BigInt(m[2]! + (m[3] ?? '').padEnd(4, '0'));
  return scaled === 0n ? 0n : sign * scaled;
}

function isValidGuardSide(minRaw: string, maxRaw: string): boolean {
  const minTrim = minRaw.trim();
  const maxTrim = maxRaw.trim();
  if (minTrim === '' && maxTrim === '') return true;
  if (minTrim === '' || maxTrim === '') return false;
  const minVal = parseSignedScale4(minTrim);
  const maxVal = parseSignedScale4(maxTrim);
  if (minVal === null || maxVal === null) return false;
  return minVal <= maxVal;
}

function parsePositiveWholeHours(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  if (BigInt(trimmed) <= 0n) return null;
  return +trimmed;
}

function isValidStaleLimit(raw: string): boolean {
  const m = /^\+?(\d+)(?:\.(\d{1,2}))?$/.exec(raw.trim());
  if (!m) return false;
  const scaled = BigInt(m[1]! + (m[2] ?? '').padEnd(2, '0'));
  return scaled > 0n;
}

export function frameErrors(
  frame: FrameFormState,
): Partial<Record<FrameErrorField, keyof typeof table.th>> {
  const errors: Partial<Record<FrameErrorField, keyof typeof table.th>> = {};
  for (const row of frame.guards) {
    if (!isValidGuardSide(row.buyMin, row.buyMax) || !isValidGuardSide(row.sellMin, row.sellMax)) {
      errors.guards = 'guardRangeInvalid';
      break;
    }
  }
  if (parsePositiveWholeHours(frame.maxReferenceAgeHours) === null) {
    errors.maxReferenceAgeHours = 'maxReferenceAgeInvalid';
  }
  if (frame.onStale === 'LIMIT' && !isValidStaleLimit(frame.staleLimitThb)) {
    errors.staleLimitThb = 'staleLimitRequired';
  }
  return errors;
}

interface GuardBandDto {
  min_pct: string;
  max_pct: string;
}

interface CurrencyGuardDto {
  buy?: GuardBandDto;
  sell?: GuardBandDto;
}

function buildGuardsPayload(rows: GuardFormRow[]): Record<string, CurrencyGuardDto> {
  const out: Record<string, CurrencyGuardDto> = {};
  for (const row of rows) {
    const buyMin = row.buyMin.trim();
    const buyMax = row.buyMax.trim();
    const sellMin = row.sellMin.trim();
    const sellMax = row.sellMax.trim();
    const entry: CurrencyGuardDto = {};
    if (buyMin !== '' && buyMax !== '') {
      entry.buy = { min_pct: buyMin, max_pct: buyMax };
    }
    if (sellMin !== '' && sellMax !== '') {
      entry.sell = { min_pct: sellMin, max_pct: sellMax };
    }
    if (entry.buy || entry.sell) {
      out[row.currency] = entry;
    }
  }
  return out;
}

type GuardValues = Omit<GuardFormRow, 'currency'>;

function emptyGuardValues(): GuardValues {
  return { buyMin: '', buyMax: '', sellMin: '', sellMax: '' };
}

function extractStoredGuards(raw: unknown): Record<string, GuardValues> {
  const out: Record<string, GuardValues> = {};
  if (!raw || typeof raw !== 'object') return out;
  for (const [code, val] of Object.entries(raw as Record<string, unknown>)) {
    const g = (val ?? {}) as {
      buy?: { min_pct?: unknown; max_pct?: unknown };
      sell?: { min_pct?: unknown; max_pct?: unknown };
    };
    out[code] = {
      buyMin: typeof g.buy?.min_pct === 'string' ? g.buy.min_pct : '',
      buyMax: typeof g.buy?.max_pct === 'string' ? g.buy.max_pct : '',
      sellMin: typeof g.sell?.min_pct === 'string' ? g.sell.min_pct : '',
      sellMax: typeof g.sell?.max_pct === 'string' ? g.sell.max_pct : '',
    };
  }
  return out;
}

export interface RatesPolicyCardProps {
  branch: () => string;
  busy: () => boolean;
  canSavePolicy: () => boolean;
  policyData: () => RatePolicyData | null;
  policyError: () => string | null;
  pubData?: () => PublishedRatesData | null;
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
  let sectionRef: HTMLDivElement | undefined;
  const [domPubCurrencies, setDomPubCurrencies] = createSignal<string[]>([]);
  const [guardMap, setGuardMap] = createSignal<Record<string, GuardValues>>({});
  const [maxReferenceAgeHours, setMaxReferenceAgeHours] = createSignal<string>('');
  const [onStale, setOnStale] = createSignal<string>('STOP');
  const [staleLimitThb, setStaleLimitThb] = createSignal<string>('');
  const [frameErrs, setFrameErrs] = createSignal<Partial<Record<FrameErrorField, keyof typeof table.th>>>({});

  const syncDomPubCurrencies = () => {
    const scope = sectionRef?.closest('.admin-rates-screen') ?? document;
    const codes: string[] = [];
    for (const el of Array.from(scope.querySelectorAll<HTMLElement>('[data-published-line]'))) {
      const code = el.getAttribute('data-published-line');
      if (code && !codes.includes(code)) codes.push(code);
    }
    setDomPubCurrencies((prev) =>
      prev.length === codes.length && prev.every((c, i) => c === codes[i]) ? prev : codes,
    );
  };

  onMount(() => {
    syncDomPubCurrencies();
    const target = sectionRef?.closest('.admin-rates-screen') ?? document.body;
    const observer = new MutationObserver(() => syncDomPubCurrencies());
    observer.observe(target, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['data-published-line'],
    });
    onCleanup(() => observer.disconnect());
  });

  createEffect(() => {
    const policy = props.policyData()?.policy;
    props.branch();
    syncDomPubCurrencies();
    queueMicrotask(syncDomPubCurrencies);
    if (policy) {
      setGuardMap(extractStoredGuards(policy.guards));
      setMaxReferenceAgeHours(policy.maxReferenceAgeHours != null ? String(policy.maxReferenceAgeHours) : '');
      setOnStale(policy.onStale ?? '');
      setStaleLimitThb(policy.staleLimitThb ?? '');
      setFrameErrs({});
    } else {
      setGuardMap({});
      setMaxReferenceAgeHours('');
      setOnStale('STOP');
      setStaleLimitThb('');
      setFrameErrs({});
    }
  });

  const guardCurrencies = createMemo<string[]>(() => {
    const fromProp = props.pubData?.()?.lines?.map((l) => l.currency);
    const pubList = fromProp ?? domPubCurrencies();
    const storedKeys = Object.keys(props.policyData()?.policy?.guards ?? {});
    const seen = new Set<string>();
    const out: string[] = [];
    for (const c of [...pubList, ...storedKeys]) {
      if (c && !seen.has(c)) {
        seen.add(c);
        out.push(c);
      }
    }
    return out;
  });

  const currentGuardRows = (): GuardFormRow[] => {
    const map = guardMap();
    return guardCurrencies().map((currency) => ({
      currency,
      ...(map[currency] ?? emptyGuardValues()),
    }));
  };

  const updateGuardField = (currency: string, field: keyof GuardValues, value: string) => {
    setGuardMap((prev) => {
      const cur = prev[currency] ?? emptyGuardValues();
      return { ...prev, [currency]: { ...cur, [field]: value } };
    });
    setFrameErrs((prev) => ({ ...prev, guards: undefined }));
  };

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

    const guardRows = currentGuardRows();
    const errs = frameErrors({
      guards: guardRows,
      maxReferenceAgeHours: maxReferenceAgeHours(),
      onStale: onStale(),
      staleLimitThb: staleLimitThb(),
    });
    setFrameErrs(errs);
    if (Object.keys(errs).length > 0) return;

    const parsedAge = parsePositiveWholeHours(maxReferenceAgeHours())!;
    const staleMode = onStale() === '' ? null : onStale();
    const staleLimit = staleMode === 'LIMIT' ? staleLimitThb().trim() : null;

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
      guards: buildGuardsPayload(guardRows),
      maxReferenceAgeHours: parsedAge,
      onStale: staleMode,
      staleLimitThb: staleLimit,
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
    <div class="rates-section" ref={sectionRef}>
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
        <Show when={frameErrs().guards}>
          {(key) => (
            <div class="form-error" role="alert" data-field-error="guards">
              {props.t(key())}
            </div>
          )}
        </Show>
        <Show when={frameErrs().maxReferenceAgeHours}>
          {(key) => (
            <div class="form-error" role="alert" data-field-error="maxReferenceAgeHours">
              {props.t(key())}
            </div>
          )}
        </Show>
        <Show when={frameErrs().staleLimitThb}>
          {(key) => (
            <div class="form-error" role="alert" data-field-error="staleLimitThb">
              {props.t(key())}
            </div>
          )}
        </Show>
      </div>

      <div class="adjust-all-bar adjust-all-bar--policy">
        <label class="adjust-field-label adjust-field-label--checkbox">
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

        <Field label={props.t('sharpMoveThresholdLabel')} class="adjust-field-label">
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
        </Field>

        <Field label={props.t('maxReferenceAgeHoursLabel')} class="adjust-field-label">
          <input
            type="text"
            name="maxReferenceAgeHours"
            value={maxReferenceAgeHours()}
            disabled={!props.canSavePolicy()}
            onInput={(e) => {
              setMaxReferenceAgeHours(e.currentTarget.value);
              setFrameErrs((prev) => ({ ...prev, maxReferenceAgeHours: undefined }));
            }}
          />
        </Field>

        <Field label={props.t('onStaleLabel')} class="adjust-field-label">
          <select
            name="onStale"
            value={onStale()}
            disabled={!props.canSavePolicy()}
            onChange={(e) => {
              setOnStale(e.currentTarget.value);
              setFrameErrs((prev) => ({ ...prev, staleLimitThb: undefined }));
            }}
          >
            <option value="STOP">{props.t('onStaleStop')}</option>
            <option value="LIMIT">{props.t('onStaleLimit')}</option>
            <option value="">{props.t('onStaleNone')}</option>
          </select>
        </Field>

        <Show when={onStale() === 'LIMIT'}>
          <Field label={props.t('staleLimitThbLabel')} class="adjust-field-label">
            <input
              type="text"
              name="staleLimitThb"
              value={staleLimitThb()}
              disabled={!props.canSavePolicy()}
              onInput={(e) => {
                setStaleLimitThb(e.currentTarget.value);
                setFrameErrs((prev) => ({ ...prev, staleLimitThb: undefined }));
              }}
            />
          </Field>
        </Show>
      </div>

      <Show when={guardCurrencies().length > 0}>
        <div class="table-scroll">
          <h3>{props.t('guardsTitle')}</h3>
          <table class="rates-table">
            <thead>
              <tr>
                <th>{props.t('headerCurrency')}</th>
                <th class="num">{`${props.t('headerBuy')} ${props.t('guardMin')}`}</th>
                <th class="num">{`${props.t('headerBuy')} ${props.t('guardMax')}`}</th>
                <th class="num">{`${props.t('headerSell')} ${props.t('guardMin')}`}</th>
                <th class="num">{`${props.t('headerSell')} ${props.t('guardMax')}`}</th>
              </tr>
            </thead>
            <tbody>
              <For each={guardCurrencies()}>
                {(code) => {
                  const row = () => guardMap()[code] ?? emptyGuardValues();
                  return (
                    <tr data-guard-line={code}>
                      <td class="code">{code}</td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardBuyMin"
                          value={row().buyMin}
                          disabled={!props.canSavePolicy()}
                          onInput={(e) => updateGuardField(code, 'buyMin', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardBuyMax"
                          value={row().buyMax}
                          disabled={!props.canSavePolicy()}
                          onInput={(e) => updateGuardField(code, 'buyMax', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardSellMin"
                          value={row().sellMin}
                          disabled={!props.canSavePolicy()}
                          onInput={(e) => updateGuardField(code, 'sellMin', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardSellMax"
                          value={row().sellMax}
                          disabled={!props.canSavePolicy()}
                          onInput={(e) => updateGuardField(code, 'sellMax', e.currentTarget.value)}
                        />
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

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
