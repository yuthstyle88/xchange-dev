// Step 4, the save result (design xchange-v3-wave3-desk.md D-2, D-3, §3.2 rows 4, §3.9 DK-1, §5.4; #543 Part 2).
// D-2: nothing receipt-like before the exchange is saved. The printed receipt is the server's stored one, read through
// TD-2's `receipt_document` (#1294) and printed by TD-3's `ReceiptFrame` (#1291); until they merge, Print prints the page.
import { Match, Show, Switch, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import type { DeskStep } from '../../steps';
import { ReferralCodeField } from '../../referral/ReferralCodeField';
import * as texts from './i18n';

const t = screenT('desk.steps.receipt', texts);

function ReceiptStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const saved = () => (store.outcome() === 'saved' ? store.saved() : undefined);
  const adminHref = () => `${store.config()?.adminUrl ?? '/'}#transactions`;
  const devicesHref = () => `/devices?branch=${encodeURIComponent(store.branch())}`;
  const newEnabled = () => store.outcome() === 'saved' || store.outcome() === 'refused';

  return (
    <section class="desk-form">
      <h2>{t('heading')}</h2>
      <Switch fallback={<p class="desk-notice">{t('waiting')}</p>}>
        <Match when={store.outcome() === 'saving'}>
          <p class="desk-notice">
            {store.transactionState() === 'PENDING_APPROVAL' ? t('pendingApproval') : t('saving')}
          </p>
        </Match>
        <Match when={saved()}>
          {(answer) => (
            <div class="desk-field">
              <p>{t('saved')}</p>
              <p>
                {t('documentRef')} <strong>{answer().documentRef ?? answer().id}</strong>
              </p>
              <a href={adminHref()}>{t('adminLink')}</a>
              <Show when={store.path.devicesLink}>
                <a href={devicesHref()}>{t('devicesLink')}</a>
              </Show>
              <ReferralCodeField transactionId={answer().id} api={store.api} />
            </div>
          )}
        </Match>
        <Match when={store.outcome() === 'refused'}>
          <p class="desk-error" role="alert">
            {store.saveError() ?? t('failed')}
          </p>
          <div class="desk-actions">
            <button type="button" onClick={() => store.backToEdit()}>
              {t('backToEdit')}
            </button>
          </div>
        </Match>
        <Match when={store.outcome() === 'unknown'}>
          <Show when={store.saveError()}>
            <p class="desk-error">{store.saveError()}</p>
          </Show>
          <div class="desk-actions">
            <button type="button" onClick={() => void store.submit()}>
              {t('retry')}
            </button>
            <span class="desk-notice">{t('unknownNote')}</span>
          </div>
        </Match>
      </Switch>
      <div class="desk-actions">
        <button type="button" disabled={!newEnabled()} onClick={() => store.startNew()}>
          {t('newExchange')}
        </button>
        <button type="button" disabled={!saved()} onClick={() => window.print()}>
          {t('print')}
        </button>
      </div>
    </section>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 4,
  label: () => t('label'),
  component: (p) => <ReceiptStep store={p.store} />,
};
