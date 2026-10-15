// TD-3 #1291: the B04 screen `/desk/transactions/{id}` (design `docs/design/blueprint/desk-transaction-detail.md` §4.5,
// D1–D11).
//
// Reads (D2): `staff_me`, `transaction?id=` and `transaction_audit?id=` once on open, on the refresh button, and
// (`transaction` and `transaction_audit`) once after the screen's own request succeeds. No timer.
// Actions go through `can()` (D8): reprint (D5) and the reversal request (D6). There is no post button.
import { createEffect, createSignal, Match, Show, Switch, type JSX } from 'solid-js';
import { ApiClient } from '../../shared/api/client';
import type { ReceiptDocumentViewData } from '../../shared/api/generated/receipt_document';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { TransactionDetailDto, TransactionViewData } from '../../shared/api/generated/transaction';
import type {
  TransactionAuditEventDto,
  TransactionAuditViewData,
} from '../../shared/api/generated/transaction_audit';
import { wordedError } from '../../shared/i18n/worded-error';
import { ReceiptFrame } from '../../shared/receipt/ReceiptFrame';
import { can } from '../staff/parts/can';
import { command } from '../staff/parts/command';
import { ReasonDialog } from '../staff/parts/ReasonDialog';
import { AuditSection } from './AuditSection';
import { HeaderSection } from './HeaderSection';
import { LinesSection } from './LinesSection';
import { PaymentSection } from './PaymentSection';
import { ReceiptsSection } from './ReceiptsSection';
import { ReversalSection } from './ReversalSection';
import { branchFromSearch, idFromPath } from './route';
import { t } from './texts';
import './desk-transaction.css';

export interface TransactionScreenProps {
  /** `window.location.pathname`, e.g. `/desk/transactions/<uuid>`. */
  pathname: string;
  /** `window.location.search`; an owner's `?branch=` is forwarded to every read. */
  search: string;
}

type State = 'loading' | 'ready' | 'not-found' | 'error';

export function TransactionScreen(props: TransactionScreenProps): JSX.Element {
  const api = new ApiClient();
  const id = idFromPath(props.pathname);
  const branch = () => branchFromSearch(props.search);

  const [state, setState] = createSignal<State>('loading');
  const [me, setMe] = createSignal<StaffMeView | undefined>(undefined);
  const [tx, setTx] = createSignal<TransactionDetailDto | null>(null);
  const [events, setEvents] = createSignal<TransactionAuditEventDto[]>([]);
  const [message, setMessage] = createSignal<string | null>(null);
  const [asking, setAsking] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  let prevBranch = branchFromSearch(props.search);
  createEffect(() => {
    const curBranch = branchFromSearch(props.search);
    if (curBranch !== prevBranch) {
      prevBranch = curBranch;
      setTx(null);
      setEvents([]);
      setMessage(null);
      void load(true);
    }
  });

  // One request command for the screen's life: it keeps its Idempotency-Key until a send succeeds (S-3 `command.ts`).
  const request =
    id === null
      ? null
      : command<{ reason: string }, unknown>(api, 'POST', `/api/transactions/${encodeURIComponent(id)}/reversal-requests`);

  /** Reads the transaction and its audit; `staff_me` too when `withMe`. */
  async function load(withMe: boolean): Promise<void> {
    if (id === null) {
      setState('not-found');
      return;
    }
    setState('loading');
    const meRead = withMe
      ? api
          .view<StaffMeView>('staff_me')
          .then((a) => a.data)
          .catch(() => undefined)
      : Promise.resolve(me());
    try {
      const [who, detail, audit] = await Promise.all([
        meRead,
        api.view<TransactionViewData>('transaction', { branch: branch(), filter: { id } }),
        api.view<TransactionAuditViewData>('transaction_audit', { branch: branch(), filter: { id } }),
      ]);
      setMe(who);
      const found = detail.data?.transaction ?? null;
      setTx(found);
      setEvents(audit.data?.events ?? []);
      setState(found === null ? 'not-found' : 'ready');
    } catch (err) {
      setMe(await meRead);
      setMessage(wordedError(err));
      setTx(null);
      setEvents([]);
      setState('error');
    }
  }

  void load(true);

  function refresh(): void {
    setMessage(null);
    void load(true);
  }

  async function reprint(number: string): Promise<void> {
    setMessage(null);
    setBusy(true);
    try {
      const answer = await api.view<ReceiptDocumentViewData>('receipt_document', { branch: branch(), filter: { number } });
      const doc = answer.data?.receipt ?? null;
      if (doc === null) {
        setMessage(t('docNotFound'));
        return;
      }
      await ReceiptFrame.print(doc.html);
    } catch (err) {
      setMessage(wordedError(err));
    } finally {
      setBusy(false);
    }
  }

  async function sendRequest(reason: string): Promise<void> {
    setAsking(false);
    if (request === null) return;
    setMessage(null);
    setBusy(true);
    try {
      await request.send({ reason });
      setMessage(t('requested'));
      await load(false);
    } catch (err) {
      setMessage(wordedError(err));
    } finally {
      setBusy(false);
    }
  }

  const canRequest = () => {
    const current = tx();
    if (current === null || current.status !== 'COMPLETED') return false;
    if (current.reversal !== null && current.reversal.status !== 'REJECTED') return false;
    return can(me(), 'ReversalRequest');
  };

  return (
    <div class="xc-dtx" data-state={state()}>
      <div class="xc-dtx-bar">
        <h1 class="xc-dtx-title">{t('title')}</h1>
        <button type="button" data-action="refresh" disabled={state() === 'loading'} onClick={refresh}>
          {t('refresh')}
        </button>
      </div>
      <Show when={message()}>
        <p class="xc-dtx-message" role="status">
          {message()}
        </p>
      </Show>
      <Switch>
        <Match when={state() === 'loading'}>
          <p class="xc-dtx-muted">{t('loading')}</p>
        </Match>
        <Match when={state() === 'not-found'}>
          <p class="xc-dtx-muted">{t('notFound')}</p>
        </Match>
        <Match when={state() === 'error'}>
          <p class="xc-dtx-muted">{t('loadFailed')}</p>
          <button type="button" data-action="retry" onClick={refresh}>
            {t('retry')}
          </button>
        </Match>
        <Match when={state() === 'ready' && tx()}>
          {(current) => (
            <>
              <HeaderSection tx={current()} />
              <LinesSection lines={current().lines} />
              <Show when={current().payment}>{(p) => <PaymentSection payment={p()} />}</Show>
              <Show when={current().receipts.length > 0}>
                <ReceiptsSection
                  receipts={current().receipts}
                  canReprint={can(me(), 'ReceiptReprint')}
                  busy={busy()}
                  onReprint={(number) => void reprint(number)}
                />
              </Show>
              <Show when={current().reversal}>{(r) => <ReversalSection reversal={r()} />}</Show>
              <Show when={canRequest()}>
                <div class="xc-dtx-actions">
                  <button type="button" data-action="request-reversal" disabled={busy()} onClick={() => setAsking(true)}>
                    {t('requestReversal')}
                  </button>
                </div>
              </Show>
              <AuditSection events={events()} />
            </>
          )}
        </Match>
      </Switch>
      <ReasonDialog
        open={asking()}
        title={t('reasonTitle')}
        onConfirm={(reason) => void sendRequest(reason)}
        onCancel={() => setAsking(false)}
      />
    </div>
  );
}
