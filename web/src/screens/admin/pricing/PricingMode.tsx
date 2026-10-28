// V.9 pricing mode panel (pricing-tiers.md §15.1, §15.8a): one currency's mode (AUTO · MANUAL · FOLLOW), its latest
// approved tier set, MAIN's FOLLOW plan result, and at MAIN the FOLLOW ⇄ MANUAL switch (pricingModeSave, PV-f4).
import { For, Show, createEffect, createSignal, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type {
  PricingModeSaveInput,
  PricingModeSaveOutput,
} from '../../../shared/api/generated/pricing_mode_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import {
  ROUTE_PRICING_MODE,
  followReasonLabel,
  followStatusLabel,
  modeLabel,
  tierSetSourceLabel,
} from './pricing-labels';
import './pricing-mode.css';

export interface PricingModeProps {
  api: ApiClient;
  branch: string;
  row: PricingRowDto;
  /** `pricing_board.isMain`: the switch exists only at MAIN (the server answers 400 elsewhere). */
  isMain: boolean;
  /** EXECUTIVE at this branch; the switch is hidden otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingModeSaveOutput): void;
}

const t = screenT('admin.pricing', table);

const BANGKOK_DT = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  day: '2-digit',
  month: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

function formatBangkok(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const parts: Record<string, string> = {};
  for (const p of BANGKOK_DT.formatToParts(d)) {
    parts[p.type] = p.value;
  }
  return `${parts.day}/${parts.month} ${parts.hour}:${parts.minute}`;
}

export function PricingMode(props: PricingModeProps): JSX.Element {
  const modeWrite = keyedWrite<PricingModeSaveInput, PricingModeSaveOutput>(
    props.api,
    'POST',
    ROUTE_PRICING_MODE,
  );
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  createEffect(() => {
    void props.row.currency;
    setError(null);
  });

  async function switchMode(mode: 'FOLLOW' | 'MANUAL'): Promise<void> {
    if (!props.isMain || !props.mayEdit || props.disabledReason || saving()) return;
    setError(null);
    setSaving(true);
    try {
      const out = await modeWrite.send({ currency: props.row.currency, mode });
      props.onSaved(out);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section class="pricing-mode">
      <div class="pricing-mode-header">
        <span class="pricing-mode-title">{t('modeTitle')}</span>
        <span class="pricing-mode-badge" data-mode={props.row.mode}>
          {modeLabel(props.row.mode)}
        </span>
      </div>

      <Show when={error()}>
        {(msg) => (
          <div role="alert" class="pricing-mode-alert">
            {msg()}
          </div>
        )}
      </Show>

      <Show when={props.mayEdit}>
        <Show
          when={props.isMain}
          fallback={<p class="pricing-mode-main-only">{t('modeMainOnly')}</p>}
        >
          <Show
            when={props.row.mode === 'FOLLOW'}
            fallback={
              <button
                type="button"
                name="mode-follow"
                class="pricing-mode-switch"
                disabled={saving() || Boolean(props.disabledReason)}
                title={props.disabledReason}
                onClick={() => void switchMode('FOLLOW')}
              >
                {t('switchToFollow')}
              </button>
            }
          >
            <button
              type="button"
              name="mode-manual"
              class="pricing-mode-switch"
              disabled={saving() || Boolean(props.disabledReason)}
              title={props.disabledReason}
              onClick={() => void switchMode('MANUAL')}
            >
              {t('switchToManual')}
            </button>
          </Show>
        </Show>
      </Show>

      <div class="pricing-mode-tier-set">
        <span class="pricing-mode-subtitle">{t('tierSetTitle')}</span>
        <Show
          when={props.row.tierSet}
          fallback={<p class="pricing-mode-empty">{t('tierSetNone')}</p>}
        >
          {(ts) => (
            <div class="pricing-mode-tier-set-body">
              <span class="pricing-mode-source">{tierSetSourceLabel(ts().source)}</span>
              <div class="pricing-mode-tiers">
                <For each={ts().tiers}>
                  {(tier) => (
                    <div class="pricing-mode-tier">
                      <span class="pricing-mode-tier-label">{t('tier', { n: tier.tierNo })}</span>
                      <span class="pricing-mode-tier-rates">
                        {tier.buy ?? '—'} / {tier.sell ?? '—'}
                      </span>
                    </div>
                  )}
                </For>
              </div>
              <p class="pricing-mode-approved">
                {t('tierSetApproved', {
                  by: ts().approvedBy,
                  at: formatBangkok(ts().approvedAt),
                })}
              </p>
            </div>
          )}
        </Show>
      </div>

      <Show when={props.row.followResult}>
        {(fr) => (
          <div class="pricing-mode-follow">
            <span class="pricing-mode-subtitle">{t('followTitle')}</span>
            <span class="pricing-mode-follow-status">{followStatusLabel(fr().status)}</span>
            <For each={fr().reasons}>
              {(reason) => <span class="pricing-mode-follow-reason">{followReasonLabel(reason)}</span>}
            </For>
            <p class="pricing-mode-follow-planned">
              {t('followPlannedAt', { at: formatBangkok(fr().plannedAt) })}
            </p>
          </div>
        )}
      </Show>
    </section>
  );
}
