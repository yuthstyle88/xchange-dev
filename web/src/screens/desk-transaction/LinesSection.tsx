// TD-3 #1291: the rate snapshot and amounts of each line (D3), every figure the server's string.
import { For, type JSX } from 'solid-js';
import type { TxLineDto } from '../../shared/api/generated/transaction';
import { formatMoney } from '../../shared/ui/MoneyText';
import { formatRate } from '../../shared/ui/rate';
import { t } from './texts';

export function sideName(side: string): string {
  if (side === 'BUY') {
    return t('buy');
  }
  return t('sell');
}

export function formatQuantity(quantity: string): string {
  const parts = quantity.split('.');
  if (parts.length === 2) {
    let frac = parts[1]!;
    while (frac.length > 2 && frac.endsWith('0')) {
      frac = frac.slice(0, -1);
    }
    return formatMoney(`${parts[0]}.${frac}`);
  }
  return formatMoney(quantity);
}

export function LinesSection(props: { lines: TxLineDto[] }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="lines">
      <h2 class="xc-dtx-heading">{t('lines')}</h2>
      <table class="xc-dtx-table">
        <thead>
          <tr>
            <th>{t('currency')}</th>
            <th>{t('side')}</th>
            <th>{t('quantity')}</th>
            <th>{t('unitSize')}</th>
            <th>{t('rateSnapshot')}</th>
            <th>{t('thbAmount')}</th>
            <th>{t('feeThb')}</th>
            <th>{t('settleThb')}</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.lines}>
            {(line) => (
              <tr>
                <td>{line.currency}</td>
                <td>{sideName(line.side)}</td>
                <td>{formatQuantity(line.quantity)}</td>
                <td>{line.unitSize}</td>
                <td>{formatRate(line.rateSnapshot, line.unitSize, line.currency)}</td>
                <td>{line.thbAmount}</td>
                <td>{line.feeThb}</td>
                <td>{line.settleThb}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </section>
  );
}
