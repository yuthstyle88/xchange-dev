// TD-3 #1291: the audit trail (D7), oldest first, as the server sent it.
import { For, Show, type JSX } from 'solid-js';
import type { TransactionAuditEventDto } from '../../shared/api/generated/transaction_audit';
import { t } from './texts';

export function AuditSection(props: { events: TransactionAuditEventDto[] }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="audit">
      <h2 class="xc-dtx-heading">{t('audit')}</h2>
      <Show when={props.events.length > 0} fallback={<p class="xc-dtx-muted">{t('noAudit')}</p>}>
        <table class="xc-dtx-table">
          <thead>
            <tr>
              <th>{t('at')}</th>
              <th>{t('actor')}</th>
              <th>{t('action')}</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.events}>
              {(e) => (
                <tr>
                  <td>{e.at}</td>
                  <td>
                    {e.actorId} · {e.actorRole}
                  </td>
                  <td>{e.action}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </section>
  );
}
