// The save of an exchange (design xchange-v3-wave3-desk.md §5.4, D-6, D-10; quote-transaction.md §4.2, §10 "Desk"):
// the uploads, then POST /api/transactions, …/confirm and …/complete, each with an Idempotency-Key. The outcome is the
// server's transaction state; a refusal is a 4xx with an error code, and anything else that fails is unknown (DK-1).
// No arithmetic and no figure the quote did not give (W7).
import { ApiClient, ApiError } from '../../shared/api/client';
import { keyedWrite } from '../../shared/api/keyed-write';
import type { TransactionViewData } from '../../shared/api/generated/transaction';
import type {
  TransactionCancelInput,
  TransactionConfirmInput,
  TransactionCreateInput,
} from '../../shared/api/generated/transactions';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import type { DeskStore, Reconfirm, SaveOutcome, TransactionState } from './desk-store';
import * as screenI18n from './i18n';

/** The completed exchange. `…/complete` (TX-5 #873) has no generated answer yet, so only `id`, `status` and
 *  `documentRef` are read from it. */
export interface SaveAnswer {
  id: string;
  status: 'COMPLETED';
  documentRef: string | null;
}

export interface StockWait {
  message: string;
}

export interface SaveSink {
  setOutcome(o: SaveOutcome): void;
  setReconfirm(r: Reconfirm | undefined): void;
  setTransactionState(s: TransactionState | undefined): void;
  setSaveError(e: string | undefined): void;
  setSaved(a: SaveAnswer | undefined): void;
  setStockWait(w: StockWait | undefined): void;
}

/** How often a transaction waiting for approval is re-read. */
export const APPROVAL_POLL_MS = 15_000;

const STATES: readonly TransactionState[] = ['DRAFT', 'PENDING_APPROVAL', 'CONFIRMED', 'COMPLETED', 'CANCELLED'];

let mintedKey = '';
const keyMinter = keyedWrite<null, void>(
  {
    command: (_method, _path, _body, key) => {
      mintedKey = key;
      return Promise.resolve();
    },
  } as ApiClient,
  'POST',
  '/',
);

export function newKey(): string {
  keyMinter.reset();
  void keyMinter.send(null);
  return mintedKey;
}

/** One exchange's progress: what has been sent and answered, so a retry redoes nothing that succeeded. */
interface Session {
  passportKey?: string;
  slipKey?: string;
  quoteId?: string;
  paidAt: string;
  tx?: { id: string; status: TransactionState; dto: Record<string, unknown> };
  completeKey?: string;
}

/** A 2xx whose body names no transaction state: the outcome is unknown. */
class NoState extends Error {}

class Refused extends Error {
  constructor(
    message: string,
    readonly code: string,
  ) {
    super(message);
  }
}

function transactionOf(body: unknown): NonNullable<Session['tx']> {
  if (body && typeof body === 'object') {
    const o = body as Record<string, unknown>;
    const status = o.status;
    if (typeof o.id === 'string' && typeof status === 'string' && (STATES as readonly string[]).includes(status)) {
      return { id: o.id, status: status as TransactionState, dto: o };
    }
  }
  throw new NoState('no transaction state');
}

function expired(expiresAt: string): boolean {
  const at = Date.parse(expiresAt);
  return Number.isNaN(at) || at <= Date.now();
}

export function createSaver(store: DeskStore, sink: SaveSink) {
  const t = screenT('desk', screenI18n);
  const routes = store.path.quoteRoutes;
  let session: Session | undefined;
  let running: Promise<SaveOutcome> | undefined;
  let watch: ReturnType<typeof setInterval> | undefined;
  let stockWait: StockWait | undefined;

  function setStockWait(w: StockWait | undefined): void {
    stockWait = w;
    sink.setStockWait(w);
  }

  const txCommandApi = {
    command: async <B,>(method: string, path: string, body: B, key: string) => {
      const answer = await store.api.command<B, unknown>(method, path, body, key);
      return transactionOf(answer);
    },
  } as unknown as ApiClient;

  const createWrite = keyedWrite<TransactionCreateInput, NonNullable<Session['tx']>>(
    txCommandApi,
    'POST',
    routes.transactions,
  );
  const confirmWrite = keyedWrite<TransactionConfirmInput, NonNullable<Session['tx']>>(
    txCommandApi,
    'POST',
    (b) => `${routes.transactions}/${b.id}/confirm`,
  );
  const completeWrite = keyedWrite<{ id: string }, NonNullable<Session['tx']>>(
    {
      command: (method, path, body, key) =>
        txCommandApi.command(method, path, body, (session!.completeKey ??= key)),
    } as ApiClient,
    'POST',
    (b) => `${routes.transactions}/${b.id}/complete`,
  );
  const cancelWrite = keyedWrite<TransactionCancelInput, NonNullable<Session['tx']>>(
    txCommandApi,
    'POST',
    (b) => `${routes.transactions}/${b.id}/cancel`,
  );

  function makeUploadWrite(slot: 'passport' | 'slip') {
    let activeFile: File | undefined;
    const w = keyedWrite<{ slot: string; name: string; size: number; type: string }, string>(
      {
        command: async (_method: string, path: string, _body: unknown, key: string) => {
          const file = activeFile!;
          const answer = await store.api.postBytes<{ uploadKey?: unknown }>(
            path,
            file,
            file.type || 'application/octet-stream',
            key,
          );
          if (!answer || typeof answer.uploadKey !== 'string') {
            throw new NoState('no upload key');
          }
          return answer.uploadKey;
        },
      } as unknown as ApiClient,
      'POST',
      store.path.upload,
    );
    return {
      send(file: File): Promise<string> {
        activeFile = file;
        return w.send({ slot, name: file.name, size: file.size, type: file.type });
      },
      reset(): void {
        w.reset();
      },
    };
  }

  const passportUploadWrite = makeUploadWrite('passport');
  const slipUploadWrite = makeUploadWrite('slip');

  function stopWatch(): void {
    if (watch !== undefined) {
      clearInterval(watch);
      clearTimeout(watch);
      watch = undefined;
    }
  }

  function reset(): void {
    stopWatch();
    session = undefined;
    passportUploadWrite.reset();
    slipUploadWrite.reset();
    createWrite.reset();
    confirmWrite.reset();
    completeWrite.reset();
    cancelWrite.reset();
  }

  function createBody(quoteId: string, s: Session): TransactionCreateInput {
    const extras = store.entry()?.payloadExtras() ?? {};
    const text = (v: unknown) => (typeof v === 'string' && v !== '' ? v : null);
    const draft = store.staffDraftId();
    return {
      quoteId,
      customerId: store.customerId(),
      bookingId: text(extras.bookingId) ?? text(extras.onlineBookingId) ?? store.heldBooking()?.bookingId ?? null,
      kioskRequestId: text(extras.kioskRequestId),
      queueTicketId: store.walkInTicket()?.id ?? null,
      staffDraftId: store.path.usesDraft && draft !== '' ? draft : null,
      passport: { uploadKey: s.passportKey ?? '' },
      payment: {
        method: store.method(),
        reference: store.ref(),
        account: store.account(),
        slip: s.slipKey ? { uploadKey: s.slipKey } : null,
        bankFeeThb: store.bankFee() === '' ? '0' : store.bankFee(),
        paidAt: s.paidAt,
      },
      valueDate: store.valueDate(),
      notes: store.notes(),
      countConfirmed: store.countConfirmed(),
    };
  }

  async function readTransaction(id: string): Promise<NonNullable<Session['tx']>> {
    const answer = await store.api.view<TransactionViewData>(routes.transactionView, { filter: { id } });
    return transactionOf(answer.data?.transaction);
  }

  /** Re-reads the transaction while it waits for approval; goes on once it is CONFIRMED (§5.4 item 4). */
  function watchApproval(id: string): void {
    stopWatch();
    watch = setInterval(() => {
      void readTransaction(id)
        .then((tx) => {
          if (!session?.tx || session.tx.id !== id || tx.status === 'PENDING_APPROVAL') return;
          stopWatch();
          session.tx = tx;
          sink.setTransactionState(tx.status);
          void submit();
        })
        .catch(() => undefined);
    }, APPROVAL_POLL_MS);
  }

  function watchStock(): void {
    stopWatch();
    watch = setTimeout(() => {
      watch = undefined;
      void submit();
    }, APPROVAL_POLL_MS);
  }

  async function advance(s: Session): Promise<SaveOutcome> {
    for (;;) {
      const tx: NonNullable<Session['tx']> = s.tx!;
      sink.setTransactionState(tx.status);
      switch (tx.status) {
        case 'DRAFT': {
          const body: TransactionConfirmInput = { id: tx.id };
          s.tx = await confirmWrite.send(body);
          break;
        }
        case 'CONFIRMED': {
          try {
            s.tx = await completeWrite.send({ id: tx.id });
          } catch (e) {
            if (e instanceof ApiError && e.status === 409 && e.code === 'stockInsufficient') {
              setStockWait({ message: e.message });
              void store.lowCash.reload();
              watchStock();
              return 'saving';
            }
            throw e;
          }
          break;
        }
        case 'COMPLETED': {
          stopWatch();
          setStockWait(undefined);
          s.tx = undefined;
          s.completeKey = undefined;
          const ref = tx.dto.documentRef;
          const answer: SaveAnswer = {
            id: tx.id,
            status: 'COMPLETED',
            documentRef: typeof ref === 'string' ? ref : null,
          };
          sink.setSaved(answer);
          sink.setOutcome('saved');
          void store.lowCash.reload();
          try {
            await store.entry()?.afterSave?.(store, answer);
          } catch {
            // the exchange is saved; the entry's own follow-up does not change that
          }
          return 'saved';
        }
        case 'PENDING_APPROVAL': {
          // waiting for the branch manager: neither saved nor refused; no new submit, no new key
          watchApproval(tx.id);
          return 'saving';
        }
        case 'CANCELLED': {
          const reason = tx.dto.cancelReason;
          reset();
          sink.setSaveError(typeof reason === 'string' && reason !== '' ? reason : t('cancelled'));
          sink.setOutcome('refused');
          return 'refused';
        }
      }
    }
  }

  async function run(): Promise<SaveOutcome> {
    sink.setOutcome('saving');
    sink.setReconfirm(undefined);
    sink.setSaveError(undefined);
    const s: Session = (session ??= { paidAt: new Date().toISOString() });

    try {
      const refusal = store.entry()?.refusal();
      if (refusal) throw new Refused(refusal, 'entryRefused');

      // 1. the uploads (today's route, raw bytes)
      if (!s.passportKey) {
        const file = store.passport();
        if (!file) throw new Refused(t('noPassport'), 'noPassport');
        s.passportKey = await passportUploadWrite.send(file);
      }
      const slipFile = store.slip();
      if (slipFile && !s.slipKey) {
        s.slipKey = await slipUploadWrite.send(slipFile);
      }

      // 2. a quote that has not expired and matches the settlement kind (D-5, CR12), then the create
      if (!s.tx) {
        setStockWait(undefined);
        let q = store.quote();
        const unusable = (x: typeof q) =>
          !x || expired(x.expiresAt) || x.cashSettlement !== (store.method() === 'Cash');
        if (!s.quoteId || q?.id !== s.quoteId || unusable(q)) {
          const held = q;
          if (unusable(q)) q = await store.requote();
          if (!q) throw new Refused(store.quoteError()?.message ?? t('noQuote'), 'noQuote');
          s.quoteId = q.id;
          if (
            held &&
            (held.lines.length !== q.lines.length ||
              held.lines.some((l, i) => l.settleThb !== q!.lines[i]?.settleThb || l.finalRate !== q!.lines[i]?.finalRate))
          ) {
            const from = held.lines[0];
            const to = q.lines[0];
            store.setPaymentChecked(false);
            store.go(3);
            sink.setReconfirm({
              fromNet: from?.settleThb ?? '',
              toNet: to?.settleThb ?? '',
              fromRate: from?.finalRate ?? '',
              toRate: to?.finalRate ?? '',
            });
            sink.setOutcome('reconfirm');
            return 'reconfirm';
          }
        }
        s.tx = await createWrite.send(createBody(s.quoteId, s));
      }

      // 3. the server's state decides the next call
      return await advance(s);
    } catch (e) {
      const stillRunning = e instanceof ApiError && e.status === 409 && e.code === 'idempotencyConflict';
      if (
        !stillRunning &&
        (e instanceof Refused || (e instanceof ApiError && e.status >= 400 && e.status < 500 && e.code !== 'httpError'))
      ) {
        // a refusal: the next submit is a new exchange
        stopWatch();
        setStockWait(undefined);
        s.tx = undefined;
        s.completeKey = undefined;
        s.quoteId = undefined;
        sink.setSaveError(e instanceof Refused ? e.message : wordedError(e));
        sink.setOutcome('refused');
        if (e instanceof ApiError && e.code === 'quoteExpired') {
          void store.requote();
        }
        if (e instanceof ApiError && e.code === 'stockInsufficient') {
          void store.lowCash.reload();
        }
        return 'refused';
      }
      // unknown: the session is kept, so a retry resends the failed request with its key
      sink.setSaveError(stillRunning ? t('stillSaving') : e instanceof NoState ? t('notConfirmed') : t('saveFailed'));
      sink.setOutcome('unknown');
      return 'unknown';
    }
  }

  async function cancelStockWait(): Promise<void> {
    const s = session;
    if (!s?.tx) return;
    stopWatch();
    try {
      s.tx = await cancelWrite.send({ id: s.tx.id, reason: t('stockCancelReason') });
      setStockWait(undefined);
      await advance(s);
    } catch (e) {
      if (stockWait) watchStock();
      sink.setSaveError(wordedError(e));
    }
  }

  function submit(): Promise<SaveOutcome> {
    running ??= run().finally(() => {
      running = undefined;
    });
    return running;
  }

  return { submit, cancelStockWait, reset, dispose: stopWatch };
}
