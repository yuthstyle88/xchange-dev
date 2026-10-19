// §9 PricingGrid: the table, enabled currencies first, others folded below (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { createSignal, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';
import type { PricingRowView } from './pricing-board.types';
import { PricingRow } from './PricingRow';
import './pricing-grid.css';

const t = screenT('admin.pricing', table);

export interface PricingGridProps {
  /** Already filtered by `Pricing`; the grid lists enabled currencies first and folds the others below. */
  rows: readonly PricingRowView[];
  selected: ReadonlySet<string>;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
}

export function PricingGrid(props: PricingGridProps): JSX.Element {
  const [showDisabled, setShowDisabled] = createSignal(false);
  const enabledRows = () => props.rows.filter((r) => r.enabled);
  const disabledRows = () => props.rows.filter((r) => !r.enabled);

  return (
    <div class="pricing-grid">
      <Show
        when={props.rows.length > 0}
        fallback={
          <div class="pricing-grid-empty" data-testid="pricing-grid-empty">
            {t('noSuggestion')}
          </div>
        }
      >
        <div class="pricing-grid-rows">
          <For each={enabledRows()}>
            {(row) => (
              <PricingRow
                row={row}
                selected={props.selected.has(row.currency)}
                onToggle={props.onToggle}
                onOpen={props.onOpen}
              />
            )}
          </For>
        </div>
        <Show when={disabledRows().length > 0}>
          <div class="pricing-grid-disabled">
            <button
              type="button"
              name="toggleDisabled"
              class="pricing-grid-toggle"
              aria-expanded={showDisabled() ? 'true' : 'false'}
              onClick={() => setShowDisabled((v) => !v)}
            >
              {t('filter')} ({disabledRows().length})
            </button>
            <Show when={showDisabled()}>
              <div class="pricing-grid-disabled-rows">
                <For each={disabledRows()}>
                  {(row) => (
                    <PricingRow
                      row={row}
                      selected={props.selected.has(row.currency)}
                      onToggle={props.onToggle}
                      onOpen={props.onOpen}
                    />
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
      </Show>
    </div>
  );
}
