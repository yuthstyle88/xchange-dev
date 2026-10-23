import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import type * as table from './i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RatePolicyData, ValueTierDto } from '../../../shared/api/generated/rate_policy';
import type { RatePolicySaveInput, RatePolicySaveOutput } from '../../../shared/api/generated/rate_policy_save';
import { fieldError } from '../../../shared/i18n/field-error';
import { Field } from '../../../shared/ui/Field';
import { checkDecimal, MONEY, numeric, RATE_PERCENT } from '../../../shared/ui/bounded';
import './rates-policy.css';

const STALE_LIMIT_BOUND = numeric(20, 2, 'nonNegative');
const SHARP_MOVE_BOUND = numeric(9, 4, 'positive');

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
  const m = /^(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!m) return null;
  const intPart = m[1]!.replace(/^0+/, '') || '0';
  const fracPart = (m[2] ?? '').replace(/0+$/, '');
  if (intPart === '0' && fracPart === '') return null;
  return { intPart, fracPart };
}

function parseTierUpper(raw: string): { intPart: string; fracPart: string } | null {
  const trimmed = raw.trim();
  if (!checkDecimal(trimmed, MONEY).ok) return null;
  return parsePositiveDecimalParts(trimmed);
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
  const n = BigInt(trimmed);
  if (n <= 0n || n > 168n) return null;
  return +trimmed;
}

function isValidStaleLimit(raw: string): boolean {
  const trimmed = raw.trim();
  if (!checkDecimal(trimmed, STALE_LIMIT_BOUND).ok) return false;
  return parsePositiveDecimalParts(trimmed) !== null;
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
  disabledReason?: string;
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
  const [sharpMoveErr, setSharpMoveErr] = createSignal<string | null>(null);
  const [maxReferenceAgeErr, setMaxReferenceAgeErr] = createSignal<string | null>(null);

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
      setSharpMoveErr(null);
      setMaxReferenceAgeErr(null);
    } else {
      setGuardMap({});
      setMaxReferenceAgeHours('');
      setOnStale('STOP');
      setStaleLimitThb('');
      setFrameErrs({});
      setSharpMoveErr(null);
      setMaxReferenceAgeErr(null);
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

  const tierIssue = (): { tierNo: 1 | 2 | 3 | 4; kind: 'nonRising' | 'upper' | 'margin' } | null => {
    if (!props.valueTiersOn()) return null;
    const rows = props.valueTiers();
    if (rows.length !== 4) return { tierNo: 1, kind: 'upper' };
    const u1Raw = rows[0]?.upperInclusiveThb ?? '';
    const u2Raw = rows[1]?.upperInclusiveThb ?? '';
    const u3Raw = rows[2]?.upperInclusiveThb ?? '';
    const u1 = parseTierUpper(u1Raw);
    const u2 = parseTierUpper(u2Raw);
    const u3 = parseTierUpper(u3Raw);

    if (u1Raw.trim() !== '' && u1 === null) return { tierNo: 1, kind: 'upper' };
    if (u2Raw.trim() !== '' && u2 === null) return { tierNo: 2, kind: 'upper' };
    if (u1 !== null && u2 !== null && !isDecimalLessThan(u1, u2)) return { tierNo: 2, kind: 'nonRising' };
    if (u3Raw.trim() !== '' && u3 === null) return { tierNo: 3, kind: 'upper' };
    if (u2 !== null && u3 !== null && !isDecimalLessThan(u2, u3)) return { tierNo: 3, kind: 'nonRising' };
    if (u1 !== null && u3 !== null && !isDecimalLessThan(u1, u3)) return { tierNo: 3, kind: 'nonRising' };

    for (const r of rows) {
      const buyTrim = r.buyMarginPct.trim();
      const sellTrim = r.sellMarginPct.trim();
      if (
        (buyTrim !== '' && !checkDecimal(buyTrim, RATE_PERCENT).ok) ||
        (sellTrim !== '' && !checkDecimal(sellTrim, RATE_PERCENT).ok)
      ) {
        return { tierNo: r.tierNo, kind: 'margin' };
      }
    }

    if (!u1) return { tierNo: 1, kind: 'upper' };
    if (!u2) return { tierNo: 2, kind: 'upper' };
    if (!u3) return { tierNo: 3, kind: 'upper' };
    if (rows[3]!.upperInclusiveThb.trim() !== '') return { tierNo: 4, kind: 'upper' };

    for (const r of rows) {
      if (
        !checkDecimal(r.buyMarginPct.trim(), RATE_PERCENT).ok ||
        !checkDecimal(r.sellMarginPct.trim(), RATE_PERCENT).ok
      ) {
        return { tierNo: r.tierNo, kind: 'margin' };
      }
    }
    return null;
  };

  const tierIssueMessage = (issue: { tierNo: 1 | 2 | 3 | 4; kind: 'nonRising' | 'upper' | 'margin' }): string => {
    if (issue.kind === 'nonRising') {
      return `${props.t('tierNonRisingPrefix')}${issue.tierNo}${props.t('tierNonRisingSuffix')}`;
    }
    if (issue.kind === 'upper') {
      return `${props.t('tierLabelPrefix')}${issue.tierNo}${props.t('tierUpperInvalidSuffix')}`;
    }
    return `${props.t('tierLabelPrefix')}${issue.tierNo}${props.t('tierMarginInvalidSuffix')}`;
  };

  const nonRisingTierNo = (): 1 | 2 | 3 | null => {
    const issue = tierIssue();
    if (!issue || issue.tierNo === 4 || issue.kind === 'margin') return null;
    return issue.tierNo;
  };

  const areValueTiersValid = (): boolean => tierIssue() === null;

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
    setMaxReferenceAgeErr(null);
    const sharpTrim = props.sharpMoveAlertPct().trim();
    if (sharpTrim !== '' && !checkDecimal(sharpTrim, SHARP_MOVE_BOUND).ok) {
      setSharpMoveErr(props.t('sharpMoveInvalid'));
      return;
    }
    setSharpMoveErr(null);
    if (Object.keys(errs).length > 0) return;

    const buyAdjChk = checkDecimal(props.adjustBuyPct().trim(), RATE_PERCENT);
    const sellAdjChk = checkDecimal(props.adjustSellPct().trim(), RATE_PERCENT);
    if (!buyAdjChk.ok || !sellAdjChk.ok) {
      props.toast(props.t('adjustPctInvalid'));
      return;
    }

    const parsedAge = parsePositiveWholeHours(maxReferenceAgeHours())!;
    const staleMode = onStale() === '' ? null : onStale();
    const staleLimit = staleMode === 'LIMIT' ? staleLimitThb().trim() : null;

    const branch = props.branch();
    const gen = props.readGen();
    const rows = props.valueTiers();
    const hasAnyTierInput = rows.some(
      (r) =>
        r.upperInclusiveThb.trim() !== '' ||
        r.buyMarginPct.trim() !== '' ||
        r.sellMarginPct.trim() !== '',
    );
    const tiersPayload: ValueTierDto[] =
      !props.valueTiersOn() && !hasAnyTierInput
        ? []
        : rows.map((r) => {
            const upper = r.upperInclusiveThb.trim();
            return {
              tierNo: r.tierNo,
              upperInclusiveThb: r.tierNo === 4 || upper === '' ? null : upper,
              buyMarginPct: r.buyMarginPct.trim(),
              sellMarginPct: r.sellMarginPct.trim(),
            };
          });

    const payload: RatePolicySaveInput & { branch: string } = {
      branch,
      buyAdjustPct: buyAdjChk.text,
      sellAdjustPct: sellAdjChk.text,
      guards: buildGuardsPayload(guardRows),
      maxReferenceAgeHours: parsedAge,
      onStale: staleMode,
      staleLimitThb: staleLimit,
      reason: policy.reason || props.t('savePolicy'),
      valueTiersOn: props.valueTiersOn(),
      valueTiers: tiersPayload,
      sharpMoveAlertPct: sharpTrim === '' ? null : sharpTrim,
    };

    try {
      await props.savePolicyWrite.send(payload);
      if (gen !== props.readGen()) return;
      props.setSimulateResult(null);
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      const fe = fieldError(err, {
        buyAdjustPct: props.t('adjustBuyPct'),
        sellAdjustPct: props.t('adjustSellPct'),
        guards: props.t('guardsTitle'),
        maxReferenceAgeHours: props.t('maxReferenceAgeHoursLabel'),
        onStale: props.t('onStaleLabel'),
        staleLimitThb: props.t('staleLimitThbLabel'),
        sharpMoveAlertPct: props.t('sharpMoveThresholdLabel'),
        valueTiers: props.t('valueTiersSwitchLabel'),
      });
      if (fe.field === 'sharpMoveAlertPct') {
        setSharpMoveErr(fe.message);
      }
      if (fe.field === 'maxReferenceAgeHours') {
        setMaxReferenceAgeErr(fe.message);
      }
      props.toast(fe.message);
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
              title={props.disabledReason}
              onClick={handleSavePolicy}
            >
              {props.t('savePolicy')}
            </button>
          </div>
        </div>
        <Show when={tierIssue()}>
          {(issue) => (
            <div
              class="form-error"
              role="alert"
              data-field-error={`tier-${issue().tierNo}`}
            >
              {tierIssueMessage(issue())}
            </div>
          )}
        </Show>
        <Show when={sharpMoveErr()}>
          {(msg) => (
            <div class="form-error" role="alert" data-field-error="sharpMoveAlertPct">
              {msg()}
            </div>
          )}
        </Show>
        <Show when={frameErrs().guards}>
          {(key) => (
            <div class="form-error" role="alert" data-field-error="guards">
              {props.t(key())}
            </div>
          )}
        </Show>
        <Show
          when={
            maxReferenceAgeErr() ??
            (frameErrs().maxReferenceAgeHours ? props.t(frameErrs().maxReferenceAgeHours!) : null)
          }
        >
          {(msg) => (
            <div class="form-error" role="alert" data-field-error="maxReferenceAgeHours">
              {msg()}
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
            title={props.disabledReason}
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
            title={props.disabledReason}
            onInput={(e) => {
              props.setSharpMoveAlertPct(e.currentTarget.value);
              setSharpMoveErr(null);
            }}
          />
        </Field>

        <Field label={props.t('maxReferenceAgeHoursLabel')} class="adjust-field-label">
          <input
            type="text"
            name="maxReferenceAgeHours"
            data-field="maxReferenceAgeHours"
            min="1"
            max="168"
            value={maxReferenceAgeHours()}
            disabled={!props.canSavePolicy()}
            title={props.disabledReason}
            onInput={(e) => {
              setMaxReferenceAgeHours(e.currentTarget.value);
              setFrameErrs((prev) => ({ ...prev, maxReferenceAgeHours: undefined }));
              setMaxReferenceAgeErr(null);
            }}
          />
        </Field>

        <Field label={props.t('onStaleLabel')} class="adjust-field-label">
          <select
            name="onStale"
            value={onStale()}
            disabled={!props.canSavePolicy()}
            title={props.disabledReason}
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
              title={props.disabledReason}
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
                          title={props.disabledReason}
                          onInput={(e) => updateGuardField(code, 'buyMin', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardBuyMax"
                          value={row().buyMax}
                          disabled={!props.canSavePolicy()}
                          title={props.disabledReason}
                          onInput={(e) => updateGuardField(code, 'buyMax', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardSellMin"
                          value={row().sellMin}
                          disabled={!props.canSavePolicy()}
                          title={props.disabledReason}
                          onInput={(e) => updateGuardField(code, 'sellMin', e.currentTarget.value)}
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name="guardSellMax"
                          value={row().sellMax}
                          disabled={!props.canSavePolicy()}
                          title={props.disabledReason}
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
                          title={props.disabledReason}
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
                        title={props.disabledReason}
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
                        title={props.disabledReason}
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
