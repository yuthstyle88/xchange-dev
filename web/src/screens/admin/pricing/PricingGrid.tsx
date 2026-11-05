// §9 PricingGrid: the currency cards, the branch's own currencies first; the ones it does not sell yet (not enabled,
// or flagged `NotOffered` by the recommendations) fold into "สกุลอื่นที่ TT / Super Rich มี (N)" below, like the shop
// board's pricing screen (owner 2026-11-05) (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { createSignal, For, Show, type JSX } from 'solid-js';
import type { PricingRecommendationDto, PricingRowDto } from '../../../shared/api/generated/pricing_board';
import { lang, screenT } from '../../../shared/i18n';
import * as table from './i18n';
import { currencyName, isUnopened, notOfferedCodes } from './pricing-format';
import { PricingRow } from './PricingRow';
import './pricing-grid.css';

const t = screenT('admin.pricing', table);

export interface PricingGridProps {
  /** Already filtered by `Pricing`; the grid lists the branch's currencies first and folds the others below. */
  rows: readonly PricingRowDto[];
  selected: ReadonlySet<string>;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
  /** The board's recommendations: a `NotOffered` currency joins the folded group (look only, no new read). */
  recommendations?: readonly PricingRecommendationDto[];
  /** The currency open in the editor, to mark its card. */
  opened?: string | null;
}

export function PricingGrid(props: PricingGridProps): JSX.Element {
  const [showDisabled, setShowDisabled] = createSignal(false);
  const notOffered = () => notOfferedCodes(props.recommendations ?? []);
  const enabledRows = () => props.rows.filter((r) => !isUnopened(r, notOffered()));
  const disabledRows = () => props.rows.filter((r) => isUnopened(r, notOffered()));
  /** `NotOffered` currencies the board has no row for: listed by code only, since there is nothing to price yet. */
  const rowless = () => notOffered().filter((c) => !props.rows.some((r) => r.currency === c));
  const foldedCount = () => disabledRows().length + rowless().length;

  const card = (row: PricingRowDto) => (
    <PricingRow
      row={row}
      selected={props.selected.has(row.currency)}
      onToggle={props.onToggle}
      onOpen={props.onOpen}
      opened={props.opened === row.currency}
    />
  );

  return (
    <div class="pricing-grid">
      <Show
        when={props.rows.length > 0 || rowless().length > 0}
        fallback={
          <div class="pricing-grid-empty" data-testid="pricing-grid-empty">
            {t('noSuggestion')}
          </div>
        }
      >
        <Show when={enabledRows().length > 0}>
          <div class="pricing-grid-rows">
            <For each={enabledRows()}>{card}</For>
          </div>
        </Show>
        <Show when={foldedCount() > 0}>
          <section class="pricing-grid-disabled">
            <button
              type="button"
              name="toggleDisabled"
              class="pricing-grid-toggle"
              aria-expanded={showDisabled() ? 'true' : 'false'}
              aria-controls="pricing-unopened"
              onClick={() => setShowDisabled((v) => !v)}
            >
              <span class="pricing-grid-caret" aria-hidden="true">
                {showDisabled() ? '▼' : '▶'}
              </span>
              <span class="pricing-grid-toggle-title">{t('unopenedTitle', { count: foldedCount() })}</span>
              <span class="pricing-grid-toggle-hint">{t('unopenedHint')}</span>
            </button>
            <Show when={showDisabled()}>
              <div class="pricing-grid-disabled-rows" id="pricing-unopened">
                <For each={disabledRows()}>{card}</For>
                <For each={rowless()}>
                  {(code) => (
                    <article class="pricing-card pricing-card-rowless" data-currency={code}>
                      <div class="pricing-card-body">
                        <header class="pricing-card-head">
                          <div class="pricing-card-title">
                            <strong class="pricing-card-code">{code}</strong>
                            <Show when={currencyName(code, lang())}>
                              {(n) => <span class="pricing-card-name">{n()}</span>}
                            </Show>
                          </div>
                        </header>
                        <p class="pricing-line-empty">{t('unopenedNoRow')}</p>
                      </div>
                    </article>
                  )}
                </For>
              </div>
            </Show>
          </section>
        </Show>
      </Show>
    </div>
  );
}
