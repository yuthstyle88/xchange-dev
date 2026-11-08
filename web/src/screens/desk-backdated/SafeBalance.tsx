// F56 (docs/design/backdated-keying.md §16.3): the SAFE balance a backdated sale posts against, shown beside the form so
// the keyer sees a shortfall before filling in a row. Skeleton by the sub-controller claude/3b81c9d5; F56b fills it.
import { createEffect, createSignal, For, onCleanup, Show, type Accessor, type JSX } from 'solid-js';
import type { ApiClient } from '../../shared/api/client';
import type { BackdateSafeData, BackdateSafeLine } from '../../shared/api/generated/backdate_safe';
import { screenT } from '../../shared/i18n';
import { formatMoney } from '../../shared/ui/MoneyText';
import { en, th } from './i18n';

export interface SafeBalanceProps {
  api: ApiClient;
  /** The branch the form keys for (`me().branch`); empty until the session is known. */
  branch: string;
  /** Bumped by the form after each save, so the balance is read again. */
  refresh: Accessor<number>;
}

const t = screenT('desk-backdated', { th, en });

/**
 * F56b: reads `views/backdate_safe` (`{ branch }`) when mounted and whenever `refresh()` changes, and shows each
 * currency as `<code> <formatMoney(amount)>` in `[data-testid="safe-balance"]`, or `[data-testid="safe-empty"]` when the
 * list is empty, with a link `[data-testid="safe-transfer-link"]` to `/staff`, where a top-up is keyed.
 */
export function SafeBalance(props: SafeBalanceProps): JSX.Element {
  const [balances, setBalances] = createSignal<BackdateSafeLine[] | null>(null);

  createEffect(() => {
    const branch = props.branch;
    props.refresh();
    if (!branch) {
      setBalances(null);
      return;
    }
    let active = true;
    props.api
      .view<BackdateSafeData>('backdate_safe', { branch })
      .then((res) => {
        if (!active) return;
        setBalances(res.data?.balances ?? []);
      })
      .catch(() => {
        // A failed read shows nothing extra and does not throw.
      });

    onCleanup(() => {
      active = false;
    });
  });

  return (
    <Show when={props.branch && balances() !== null}>
      <div>
        <div class="desk-rate-line">
          <span>{t('safeTitle')}</span>
          <a class="xc-btn xc-btn--secondary" data-testid="safe-transfer-link" href="/staff">
            {t('safeTransfer')}
          </a>
        </div>
        <Show
          when={balances()!.length > 0}
          fallback={<div data-testid="safe-empty" class="desk-empty">{t('safeEmpty')}</div>}
        >
          <div data-testid="safe-balance">
            <For each={balances()!}>
              {(b) => (
                <div class="desk-rate-line">
                  <span>{b.currency}</span>{' '}
                  <strong>{formatMoney(b.amount)}</strong>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
}
