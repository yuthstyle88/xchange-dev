// TD-3 #1291: the receipts and their reprint (D5). Revision 1 is the receipt; a later revision is the refund document.
import { For, Show, type JSX } from 'solid-js';
import type { TxReceiptDto } from '../../shared/api/generated/transaction';
import { t } from './texts';

export function ReceiptsSection(props: {
  receipts: TxReceiptDto[];
  canReprint: boolean;
  disabledReason?: string;
  busy: boolean;
  onReprint: (number: string) => void;
}): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="receipts">
      <h2 class="xc-dtx-heading">{t('receipts')}</h2>
      <table class="xc-dtx-table">
        <thead>
          <tr>
            <th>{t('number')}</th>
            <th>{t('revision')}</th>
            <th>{t('issuedAt')}</th>
            <th />
          </tr>
        </thead>
        <tbody>
          <For each={props.receipts}>
            {(r) => (
              <tr>
                <td>{r.number}</td>
                <td>
                  {r.revision} · {r.revision === 1 ? t('receipt') : t('cancellationDoc')}
                </td>
                <td>{r.issuedAt}</td>
                <td>
                  <Show when={props.canReprint || Boolean(props.disabledReason)}>
                    <button
                      type="button"
                      data-action="reprint"
                      data-number={r.number}
                      disabled={props.busy || Boolean(props.disabledReason)}
                      title={props.disabledReason}
                      onClick={() => {
                        if (!props.disabledReason) props.onReprint(r.number);
                      }}
                    >
                      {t('reprint')}
                    </button>
                  </Show>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </section>
  );
}
