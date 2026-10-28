// Step 4, the save result (design xchange-v3-wave3-desk.md D-2, D-3, §3.2 rows 4, §3.9 DK-1, §5.4; #543 Part 2).
// D-2: nothing receipt-like before the exchange is saved. The printed receipt is the server's stored one, read through
// TD-2's `receipt_document` (#1294) and printed by TD-3's `ReceiptFrame` (#1291).
import { createEffect, createSignal, Match, Show, Switch, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import type { DeskStep } from '../../steps';
import { ReferralCodeField } from '../../referral/ReferralCodeField';
import * as texts from './i18n';
import { printSaved } from './print';

const t = screenT('desk.steps.receipt', texts);

export type PrintState = 'idle' | 'printing' | 'printed' | 'failed';

function ReceiptStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const saved = () => (store.outcome() === 'saved' ? store.saved() : undefined);
  const adminHref = () => `${store.config()?.adminUrl ?? '/'}#transactions`;
  const devicesHref = () => `/devices?branch=${encodeURIComponent(store.branch())}`;
  const newEnabled = () => store.outcome() === 'saved' || store.outcome() === 'refused';

  const [printState, setPrintState] = createSignal<PrintState>('idle');
  const [printReason, setPrintReason] = createSignal('');
  let printSeq = 0;

  createEffect(() => {
    if (store.outcome() !== 'saved') {
      printSeq += 1;
      setPrintState('idle');
      setPrintReason('');
    }
  });

  function onStartNew(): void {
    printSeq += 1;
    setPrintState('idle');
    setPrintReason('');
    store.startNew();
  }

  async function onPrint(): Promise<void> {
    const answer = saved();
    if (!answer || printState() === 'printing') return;
    const seq = ++printSeq;
    setPrintState('printing');
    setPrintReason('');
    const res = await printSaved(store.api, answer.documentRef ?? answer.id);
    if (seq !== printSeq) return;
    if (res === 'printed') {
      setPrintState('printed');
    } else {
      setPrintReason(res.failed);
      setPrintState('failed');
    }
  }

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
              <Show when={printState() === 'printing'}>
                <p class="desk-notice">{t('printing')}</p>
              </Show>
              <Show when={printState() === 'printed'}>
                <p class="desk-notice">{t('printed')}</p>
              </Show>
              <Show when={printState() === 'failed'}>
                <p class="desk-error" role="alert">
                  {t('printFailed', { reason: printReason() })}
                </p>
              </Show>
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
        <button type="button" disabled={!newEnabled()} onClick={onStartNew}>
          {t('newExchange')}
        </button>
        <button
          type="button"
          disabled={!saved() || printState() === 'printing'}
          onClick={() => void onPrint()}
        >
          {saved() && (printState() === 'printed' || printState() === 'failed')
            ? t('printAgain')
            : t('print')}
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
