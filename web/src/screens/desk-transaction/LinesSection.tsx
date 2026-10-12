// TD-3 #1291: the rate snapshot and amounts of each line (D3), every figure the server's string.
import { For, type JSX } from 'solid-js';
import type { TxLineDto } from '../../shared/api/generated/transaction';
import { t } from './texts';

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
                <td>{line.side}</td>
                <td>{line.quantity}</td>
                <td>{line.unitSize}</td>
                <td>{line.rateSnapshot}</td>
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
