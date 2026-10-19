// §9 OrderRefs: recent orders with the market they were priced against (FE pricing-board PB-9, #3765; docs/design/blueprint/pricing-tiers.md §9).
import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';
import type { OrderRefView } from './pricing-board.types';
import './order-refs.css';

const t = screenT('admin.pricing', table);

export interface OrderRefsProps {
  refs: readonly OrderRefView[];
}

export function OrderRefs(props: OrderRefsProps): JSX.Element {
  return (
    <Show when={props.refs && props.refs.length > 0}>
      <section class="order-refs">
        <h3 class="order-refs-heading">{t('orderRefs')}</h3>
        <div class="order-refs-list">
          <For each={props.refs}>
            {(ref) => (
              <div class="order-ref-item">
                <span class="order-ref-doc">{ref.documentRef ?? ref.transactionId}</span>
                <span class="order-ref-time">{ref.recordedAt}</span>
                <span class="order-ref-currency">{ref.currency}</span>
                <span class="order-ref-market">
                  <Show when={ref.market && ref.market.length > 0} fallback="—">
                    <For each={ref.market}>
                      {(q) => (
                        <span class="order-ref-quote">
                          <span class="order-ref-source">{q.source}</span>{' '}
                          <span class="order-ref-rates">
                            {q.buy ?? '—'} / {q.sell ?? '—'}
                          </span>
                        </span>
                      )}
                    </For>
                  </Show>
                </span>
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}
