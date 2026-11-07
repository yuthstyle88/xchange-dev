// V.9 rate-board display settings (pricing-tiers.md §15.8): tiers shown, featured currencies, highlight and
// "all market", saved with rateDisplaySave (PV-j); `{clear: true}` drops this branch's own row.
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingDisplayDto } from '../../../shared/api/generated/pricing_board';
import type {
  RateDisplaySaveInput,
  RateDisplaySaveOutput,
} from '../../../shared/api/generated/rate_display_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import {
  DISPLAY_FEATURED_MAX,
  displayFromLabel,
  ROUTE_PRICING_DISPLAY,
  type TierNo,
} from './pricing-labels';
import './display-settings.css';

export interface DisplaySettingsProps {
  api: ApiClient;
  branch: string;
  /** `pricing_board.display`; `from` says whose row it is: `branch` · `MAIN` · `default`. */
  value: PricingDisplayDto;
  /** The board's currency codes, to pick featured ones from. */
  currencies: readonly string[];
  /** EXECUTIVE at this branch; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: RateDisplaySaveOutput): void;
}

const TIERS: readonly TierNo[] = [1, 2, 3, 4];

export function DisplaySettings(props: DisplaySettingsProps): JSX.Element {
  const t = screenT('admin.pricing', table);
  const saveWrite = keyedWrite<RateDisplaySaveInput, RateDisplaySaveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_DISPLAY,
  );

  const [tiers, setTiers] = createSignal<number[]>([...props.value.tiers]);
  const [featured, setFeatured] = createSignal<string[]>([...props.value.featured]);
  const [highlight, setHighlight] = createSignal<boolean>(props.value.highlight);
  const [allMarket, setAllMarket] = createSignal<boolean>(props.value.allMarket);
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    setTiers([...props.value.tiers]);
    setFeatured([...props.value.featured]);
    setHighlight(props.value.highlight);
    setAllMarket(props.value.allMarket);
    setError(null);
  });

  const toggleTier = (n: number, checked: boolean) => {
    setTiers((prev) => (checked ? [...prev.filter((x) => x !== n), n] : prev.filter((x) => x !== n)));
  };

  const addFeatured = (code: string) => {
    if (!code || featured().length >= DISPLAY_FEATURED_MAX || featured().includes(code)) return;
    setFeatured((prev) => [...prev, code]);
  };

  const removeFeatured = (code: string) => {
    setFeatured((prev) => prev.filter((c) => c !== code));
  };

  const send = async (body: RateDisplaySaveInput) => {
    if (!props.mayEdit || saving()) return;
    setError(null);
    setSaving(true);
    try {
      const out = await saveWrite.send(body);
      props.onSaved(out);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  };

  const handleSave = () => {
    const sortedTiers = [...tiers()].sort((a, b) => a - b);
    if (sortedTiers.length === 0) {
      setError(t('displayTiersRequired'));
      return;
    }
    void send({
      tiers: sortedTiers,
      featured: [...featured()],
      highlight: highlight(),
      allMarket: allMarket(),
    });
  };

  const handleClear = () => {
    void send({ clear: true });
  };

  return (
    <section class="display-settings">
      <header>
        <h3>{t('displayTitle')}</h3>
        <span>{displayFromLabel(props.value.from)}</span>
      </header>

      <Show when={error()}>{(msg) => <p role="alert">{msg()}</p>}</Show>

      <fieldset>
        <legend>{t('displayTiers')}</legend>
        <For each={TIERS}>
          {(n) => (
            <label>
              <input
                type="checkbox"
                name="tier"
                value={String(n)}
                checked={tiers().includes(n)}
                disabled={!props.mayEdit || saving()}
                onChange={(e) => toggleTier(n, e.currentTarget.checked)}
              />
              {t('tier', { n })}
            </label>
          )}
        </For>
      </fieldset>

      <div>
        <span>{t('displayFeatured', { max: DISPLAY_FEATURED_MAX })}</span>
        <ul>
          <For each={featured()}>
            {(code) => (
              <li>
                <span>{code}</span>
                <button
                  type="button"
                  name="featured-remove"
                  data-code={code}
                  disabled={!props.mayEdit || saving()}
                  onClick={() => removeFeatured(code)}
                >
                  ×
                </button>
              </li>
            )}
          </For>
        </ul>
        <select
          name="featured-add"
          aria-label={t('displayFeaturedAdd')}
          value=""
          disabled={!props.mayEdit || saving() || featured().length >= DISPLAY_FEATURED_MAX}
          onChange={(e) => {
            const val = e.currentTarget.value;
            e.currentTarget.value = '';
            addFeatured(val);
          }}
        >
          <option value="">{t('displayFeaturedAdd')}</option>
          <For each={props.currencies.filter((c) => !featured().includes(c))}>
            {(c) => <option value={c}>{c}</option>}
          </For>
        </select>
      </div>

      <label>
        <input
          type="checkbox"
          name="highlight"
          checked={highlight()}
          disabled={!props.mayEdit || saving()}
          onChange={(e) => setHighlight(e.currentTarget.checked)}
        />
        {t('displayHighlight')}
      </label>

      <label>
        <input
          type="checkbox"
          name="allMarket"
          checked={allMarket()}
          disabled={!props.mayEdit || saving()}
          onChange={(e) => setAllMarket(e.currentTarget.checked)}
        />
        {t('displayAllMarket')}
      </label>

      <Show when={props.mayEdit}>
        <div>
          <button
            type="button"
            name="save-display"
            disabled={Boolean(props.disabledReason) || saving()}
            title={props.disabledReason}
            onClick={handleSave}
          >
            {t('save')}
          </button>
          <Show when={props.value.from === 'branch'}>
            <button
              type="button"
              name="clear-display"
              disabled={Boolean(props.disabledReason) || saving()}
              title={props.disabledReason}
              onClick={handleClear}
            >
              {t('displayClear')}
            </button>
          </Show>
        </div>
      </Show>
    </section>
  );
}
