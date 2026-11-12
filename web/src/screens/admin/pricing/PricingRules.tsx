// The Owner's pricing rules card (owner-set-values.md §2, xchange#6555): loads `pricing_rules`, shows each rule or
// "not set" with what that means, who saved the current version and when, and the earlier versions. EXECUTIVE
// (`pricingRulesSave`) edits and saves a new version; everyone else reads. An empty box saves as "not set".
import { createEffect, createSignal, For, Index, onCleanup, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type {
  PricingRulesDto,
  PricingRulesVersionDto,
  PricingRulesViewData,
} from '../../../shared/api/generated/pricing_rules';
import type { PricingRulesSaveInput } from '../../../shared/api/generated/pricing_rules_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { formatBangkokDateTime } from '../../../shared/format/bangkok-time';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import { ROUTE_PRICING_RULES, VIEW_PRICING_RULES } from './pricing-labels';

export interface PricingRulesProps {
  api: ApiClient;
  branch: string;
  /** `pricingRulesSave` in `me.permissions`; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
}

/** The plain-number rules, in card order. */
const SINGLE = [
  'farFromAnchorLimitPct',
  'largeMovePct',
  'thbCashStep',
  'branchBpCap',
  'marketStaleAfterMinutes',
] as const;
type Single = (typeof SINGLE)[number];
const RANGES = ['betterPctRange', 'minSpreadPctRange'] as const;
type RangeField = (typeof RANGES)[number];
const ADVICE = ['farPct', 'spreadRatio', 'spreadGapPt', 'marketMovedPct'] as const;
type AdviceKey = (typeof ADVICE)[number];
type Key = keyof typeof table.th;

type Band = { fromPrice: string; decimals: string };
interface Draft {
  single: Record<Single, string>;
  ranges: Record<RangeField, { min: string; max: string }>;
  advice: Record<AdviceKey, string>;
  bands: Band[];
}

function draftOf(r: PricingRulesDto): Draft {
  const str = (v: string | number | null) => (v === null ? '' : String(v));
  return {
    single: {
      farFromAnchorLimitPct: str(r.farFromAnchorLimitPct),
      largeMovePct: str(r.largeMovePct),
      thbCashStep: str(r.thbCashStep),
      branchBpCap: str(r.branchBpCap),
      marketStaleAfterMinutes: str(r.marketStaleAfterMinutes),
    },
    ranges: {
      betterPctRange: { min: r.betterPctRange?.min ?? '', max: r.betterPctRange?.max ?? '' },
      minSpreadPctRange: { min: r.minSpreadPctRange?.min ?? '', max: r.minSpreadPctRange?.max ?? '' },
    },
    advice: {
      farPct: r.advice?.farPct ?? '',
      spreadRatio: r.advice?.spreadRatio ?? '',
      spreadGapPt: r.advice?.spreadGapPt ?? '',
      marketMovedPct: r.advice?.marketMovedPct ?? '',
    },
    bands: (r.rateDecimals ?? []).map((b) => ({ fromPrice: b.fromPrice, decimals: String(b.decimals) })),
  };
}

/** Every rule is sent, so the version saved is exactly the card: an empty box (or group) is `null`, "not set". */
function inputOf(d: Draft): PricingRulesSaveInput {
  const text = (s: string) => (s.trim() === '' ? null : s.trim());
  const int = (s: string) => (s.trim() === '' ? null : Number(s.trim()));
  const range = (r: { min: string; max: string }) =>
    r.min.trim() === '' && r.max.trim() === '' ? null : { min: r.min.trim(), max: r.max.trim() };
  const adviceEmpty = ADVICE.every((k) => d.advice[k].trim() === '');
  return {
    farFromAnchorLimitPct: text(d.single.farFromAnchorLimitPct),
    largeMovePct: text(d.single.largeMovePct),
    thbCashStep: text(d.single.thbCashStep),
    branchBpCap: int(d.single.branchBpCap),
    marketStaleAfterMinutes: int(d.single.marketStaleAfterMinutes),
    betterPctRange: range(d.ranges.betterPctRange),
    minSpreadPctRange: range(d.ranges.minSpreadPctRange),
    advice: adviceEmpty
      ? null
      : {
          farPct: d.advice.farPct.trim(),
          spreadRatio: d.advice.spreadRatio.trim(),
          spreadGapPt: d.advice.spreadGapPt.trim(),
          marketMovedPct: d.advice.marketMovedPct.trim(),
        },
    rateDecimals:
      d.bands.length === 0
        ? null
        : d.bands.map((b) => ({ fromPrice: b.fromPrice.trim(), decimals: Number(b.decimals.trim()) })),
  };
}

export function PricingRules(props: PricingRulesProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const saveWrite = keyedWrite<PricingRulesSaveInput, PricingRulesVersionDto>(
    props.api,
    'POST',
    ROUTE_PRICING_RULES,
  );

  const [data, setData] = createSignal<PricingRulesViewData | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [error, setError] = createSignal<{ field?: string; message: string } | null>(null);
  const [saving, setSaving] = createSignal(false);

  let alive = true;
  onCleanup(() => (alive = false));

  async function load(branch: string): Promise<void> {
    setLoading(true);
    setLoadError(null);
    try {
      const ans = await props.api.view<PricingRulesViewData>(VIEW_PRICING_RULES, { branch });
      if (!alive) return;
      if (ans.data !== undefined) {
        setData(ans.data);
        setDraft(draftOf(ans.data.current.rules));
      }
    } catch (err: unknown) {
      if (alive) setLoadError(wordedError(err));
    } finally {
      if (alive) setLoading(false);
    }
  }

  createEffect(() => void load(props.branch));

  const labels = (): Record<string, string> => ({
    farFromAnchorLimitPct: t('rules.farFromAnchorLimitPct'),
    largeMovePct: t('rules.largeMovePct'),
    rateDecimals: t('rules.rateDecimals'),
    thbCashStep: t('rules.thbCashStep'),
    branchBpCap: t('rules.branchBpCap'),
    betterPctRange: t('rules.betterPctRange'),
    minSpreadPctRange: t('rules.minSpreadPctRange'),
    marketStaleAfterMinutes: t('rules.marketStaleAfterMinutes'),
    advice: t('rules.advice'),
  });

  const edit = (fn: (d: Draft) => void) => {
    const d = draft();
    if (!d) return;
    const next: Draft = structuredClone(d);
    fn(next);
    setDraft(next);
  };

  const readOnly = () => !props.mayEdit || saving();

  const handleSave = async () => {
    const d = draft();
    if (!props.mayEdit || saving() || !d) return;
    setError(null);
    setSaving(true);
    try {
      await saveWrite.send(inputOf(d));
      await load(props.branch);
    } catch (err: unknown) {
      setError(fieldError(err, labels()));
    } finally {
      setSaving(false);
    }
  };

  const savedBy = (v: PricingRulesVersionDto) =>
    t('rules.savedBy', {
      name: v.setByName ?? v.setBy ?? '',
      at: v.createdAt ? formatBangkokDateTime(v.createdAt) : '',
    });

  const box = (name: string, value: string, set: (v: string) => void, label: string): JSX.Element => (
    <input
      type="text"
      inputmode="decimal"
      name={name}
      aria-label={label}
      placeholder={t('rules.notSet')}
      value={value}
      readOnly={readOnly()}
      aria-invalid={error()?.field === name.split('.')[0] ? 'true' : undefined}
      onInput={(e) => set(e.currentTarget.value)}
    />
  );

  const notSetHint = (field: string, isSet: boolean) => (
    <Show when={!isSet}>
      <small data-not-set={field}>{t('rules.notSet')}: {t(`rules.notSetMeans.${field}` as Key)}</small>
    </Show>
  );

  return (
    <section aria-label={t('rules.title')}>
      <header>
        <h3>{t('rules.title')}</h3>
      </header>

      <Show when={loading() && !data()}>
        <p>{t('loading')}</p>
      </Show>
      <Show when={loadError()}>
        {(msg) => (
          <div>
            <p role="alert">{msg()}</p>
            <button type="button" name="retry-rules" disabled={loading()} onClick={() => void load(props.branch)}>
              {t('rules.retry')}
            </button>
          </div>
        )}
      </Show>

      <Show when={data() && draft()}>
        <p data-current>
          {data()!.current.version === null ? t('rules.neverSaved') : savedBy(data()!.current)}
        </p>
        <Show when={!props.mayEdit}>
          <p>{t('rules.readOnly')}</p>
        </Show>
        <Show when={error()}>{(e) => <p role="alert">{e().message}</p>}</Show>

        <For each={SINGLE}>
          {(f) => (
            <div data-rule={f}>
              <label>
                <span>{labels()[f]}</span>
                {box(f, draft()!.single[f], (v) => edit((d) => (d.single[f] = v)), labels()[f])}
              </label>
              {notSetHint(f, draft()!.single[f].trim() !== '')}
            </div>
          )}
        </For>

        <For each={RANGES}>
          {(f) => (
            <fieldset data-rule={f}>
              <legend>{labels()[f]}</legend>
              {box(`${f}.min`, draft()!.ranges[f].min, (v) => edit((d) => (d.ranges[f].min = v)), `${labels()[f]} ${t('rules.min')}`)}
              {box(`${f}.max`, draft()!.ranges[f].max, (v) => edit((d) => (d.ranges[f].max = v)), `${labels()[f]} ${t('rules.max')}`)}
              {notSetHint(f, draft()!.ranges[f].min.trim() !== '' || draft()!.ranges[f].max.trim() !== '')}
            </fieldset>
          )}
        </For>

        <fieldset data-rule="rateDecimals">
          <legend>{labels().rateDecimals}</legend>
          <Index each={draft()!.bands}>
            {(b, i) => (
              <div>
                {box(`rateDecimals.${i}.fromPrice`, b().fromPrice, (v) => edit((d) => (d.bands[i].fromPrice = v)), `${t('rules.fromPrice')} ${i + 1}`)}
                {box(`rateDecimals.${i}.decimals`, b().decimals, (v) => edit((d) => (d.bands[i].decimals = v)), `${t('rules.decimals')} ${i + 1}`)}
                <Show when={props.mayEdit}>
                  <button
                    type="button"
                    name="band-remove"
                    disabled={saving()}
                    onClick={() => edit((d) => d.bands.splice(i, 1))}
                  >
                    {t('rules.bandRemove')}
                  </button>
                </Show>
              </div>
            )}
          </Index>
          <Show when={props.mayEdit}>
            <button
              type="button"
              name="band-add"
              disabled={saving()}
              onClick={() =>
                edit((d) => d.bands.push({ fromPrice: d.bands.length === 0 ? '0' : '', decimals: '' }))
              }
            >
              {t('rules.bandAdd')}
            </button>
          </Show>
          {notSetHint('rateDecimals', draft()!.bands.length > 0)}
        </fieldset>

        <fieldset data-rule="advice">
          <legend>{labels().advice}</legend>
          <For each={ADVICE}>
            {(k) => (
              <label>
                <span>{t(`rules.advice.${k}` as Key)}</span>
                {box(`advice.${k}`, draft()!.advice[k], (v) => edit((d) => (d.advice[k] = v)), t(`rules.advice.${k}` as Key))}
              </label>
            )}
          </For>
          {notSetHint('advice', ADVICE.some((k) => draft()!.advice[k].trim() !== ''))}
        </fieldset>

        <Show when={props.mayEdit}>
          <button
            type="button"
            name="save-rules"
            disabled={Boolean(props.disabledReason) || saving()}
            title={props.disabledReason}
            onClick={() => void handleSave()}
          >
            {t('save')}
          </button>
        </Show>

        <section aria-label={t('rules.history')}>
          <h4>{t('rules.history')}</h4>
          <Show when={data()!.history.length > 0} fallback={<p>{t('rules.historyEmpty')}</p>}>
            <ol>
              <For each={data()!.history}>{(v) => <li data-version={v.version ?? ''}>{savedBy(v)}</li>}</For>
            </ol>
          </Show>
        </section>
      </Show>
    </section>
  );
}
