// The save of an exchange (design xchange-v3-wave3-desk.md §5.4, D-6, D-10; quote-transaction.md §4.2, §10 "Desk"):
// the uploads, then POST /api/transactions, …/confirm and …/complete, each with an Idempotency-Key. The outcome is the
// server's transaction state; a refusal is a 4xx with an error code, and anything else that fails is unknown (DK-1).
// No arithmetic and no figure the quote did not give (W7).
import { ApiClient, ApiError } from '../../shared/api/client';
import { keyedWrite } from '../../shared/api/keyed-write';
import type {
  TransactionConfirmInput,
  TransactionCreateInput,
} from '../../shared/api/generated/transactions';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import type { DeskStore, SaveOutcome, TransactionState } from './desk-store';
import * as screenI18n from './i18n';

/** The completed exchange. `…/complete` (TX-5 #873) has no generated answer yet, so only `id`, `status` and
 *  `documentRef` are read from it. */
export interface SaveAnswer {
  id: string;
  status: 'COMPLETED';
  documentRef: string | null;
}

export interface SaveSink {
  setOutcome(o: SaveOutcome): void;
  setTransactionState(s: TransactionState | undefined): void;
  setSaveError(e: string | undefined): void;
  setSaved(a: SaveAnswer | undefined): void;
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
    txCommandApi,
    'POST',
    (b) => `${routes.transactions}/${b.id}/complete`,
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
  }

  function createBody(quoteId: string, s: Session): TransactionCreateInput {
    const extras = store.entry()?.payloadExtras() ?? {};
    const text = (v: unknown) => (typeof v === 'string' && v !== '' ? v : null);
    const draft = store.staffDraftId();
    return {
      quoteId,
      customerId: store.customerId(),
      bookingId: text(extras.bookingId) ?? text(extras.onlineBookingId),
      kioskRequestId: text(extras.kioskRequestId),
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

  /** Re-reads the transaction while it waits for approval; goes on once it is CONFIRMED (§5.4 item 4). */
  function watchApproval(id: string): void {
    stopWatch();
    watch = setInterval(() => {
      void store.api
        .view<{ id?: string; status?: string }>(routes.transactionView, { filter: { id } })
        .then((answer) => {
          const status = answer.data?.status;
          if (!session?.tx || session.tx.id !== id || typeof status !== 'string') return;
          if (status === 'PENDING_APPROVAL' || !(STATES as readonly string[]).includes(status)) return;
          stopWatch();
          session.tx = { ...session.tx, status: status as TransactionState };
          sink.setTransactionState(session.tx.status);
          void submit();
        })
        .catch(() => undefined);
    }, APPROVAL_POLL_MS);
  }

  async function run(): Promise<SaveOutcome> {
    sink.setOutcome('saving');
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

      // 2. a quote that has not expired (D-5), then the create
      if (!s.tx) {
        let q = store.quote();
        if (!s.quoteId || !q || q.id !== s.quoteId || expired(q.expiresAt)) {
          if (!q || expired(q.expiresAt)) q = await store.requote();
          if (!q) throw new Refused(store.quoteError()?.message ?? t('noQuote'), 'noQuote');
          s.quoteId = q.id;
        }
        s.tx = await createWrite.send(createBody(s.quoteId, s));
      }

      // 3. the server's state decides the next call
      for (;;) {
        const tx: NonNullable<Session['tx']> = s.tx;
        sink.setTransactionState(tx.status);
        switch (tx.status) {
          case 'DRAFT': {
            const body: TransactionConfirmInput = { id: tx.id };
            s.tx = await confirmWrite.send(body);
            break;
          }
          case 'CONFIRMED': {
            s.tx = await completeWrite.send({ id: tx.id });
            break;
          }
          case 'COMPLETED': {
            stopWatch();
            s.tx = undefined;
            const ref = tx.dto.documentRef;
            const answer: SaveAnswer = {
              id: tx.id,
              status: 'COMPLETED',
              documentRef: typeof ref === 'string' ? ref : null,
            };
            sink.setSaved(answer);
            sink.setOutcome('saved');
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
    } catch (e) {
      if (e instanceof Refused || (e instanceof ApiError && e.status >= 400 && e.status < 500 && e.code !== 'httpError')) {
        // a refusal: the next submit is a new exchange
        stopWatch();
        s.tx = undefined;
        s.quoteId = undefined;
        sink.setSaveError(e instanceof Refused ? e.message : wordedError(e));
        sink.setOutcome('refused');
        if (e instanceof ApiError && e.code === 'quoteExpired') {
          void store.requote();
        }
        return 'refused';
      }
      // unknown: the session is kept, so a retry resends the failed request with its key
      sink.setSaveError(e instanceof NoState ? t('notConfirmed') : t('saveFailed'));
      sink.setOutcome('unknown');
      return 'unknown';
    }
  }

  function submit(): Promise<SaveOutcome> {
    running ??= run().finally(() => {
      running = undefined;
    });
    return running;
  }

  return { submit, reset, dispose: stopWatch };
}
