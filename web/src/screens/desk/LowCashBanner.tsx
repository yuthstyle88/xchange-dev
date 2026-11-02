import { Show, type JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import * as screenI18n from './i18n';

const t = screenT('desk', screenI18n);

export function LowCashBanner(props: { lowCash: { low(): string[] } }): JSX.Element {
  const codes = () => props.lowCash.low();
  return (
    <Show when={codes().length > 0}>
      <div class="desk-warning" data-testid="low-cash" role="status">
        <p>{t('lowCashTitle')}</p>
        <p>{t('lowCashCurrencies', { currencies: codes().join(', ') })}</p>
      </div>
    </Show>
  );
}
