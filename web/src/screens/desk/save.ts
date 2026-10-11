// The save of an exchange (design xchange-v3-wave3-desk.md §5.4, D-6, D-10; quote-transaction.md §4.2, §10 "Desk"):
// the uploads, then POST /api/transactions, …/confirm and …/complete, each with an Idempotency-Key. The outcome is the
// server's transaction state; a refusal is a 4xx with an error code, and anything else that fails is unknown (DK-1).
// No arithmetic and no figure the quote did not give (W7).
import { ApiError } from '../../shared/api/client';
import type {
  TransactionConfirmInput,
  TransactionCreateInput,
} from '../../shared/api/generated/transactions';
import { screenT } from '../../shared/i18n';
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

export function newKey(): string {
  return crypto.randomUUID();
}

/** A request made once and resent byte for byte on a retry (D-10). */
interface Request {
  path: string;
  body: unknown;
  key: string;
}

/** One exchange's progress: what has been sent and answered, so a retry redoes nothing that succeeded. */
interface Session {
  passportUploadKey: string;
  passportKey?: string;
  slipUploadKey: string;
  slipKey?: string;
  create?: Request;
  confirm?: Request;
  complete?: Request;
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

  function stopWatch(): void {
    if (watch !== undefined) {
      clearInterval(watch);
      watch = undefined;
    }
  }

  function reset(): void {
    stopWatch();
    session = undefined;
  }

  async function send(r: Request): Promise<NonNullable<Session['tx']>> {
    const answer = await store.api.command<unknown, unknown>('POST', r.path, r.body, r.key);
    return transactionOf(answer);
  }

  async function upload(file: File, key: string): Promise<string> {
    const answer = await store.api.postBytes<{ uploadKey?: unknown }>(
      store.path.upload,
      file,
      file.type || 'application/octet-stream',
      key,
    );
    if (!answer || typeof answer.uploadKey !== 'string') {
      throw new NoState('no upload key');
    }
    return answer.uploadKey;
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
        paidAt: new Date().toISOString(),
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
    const s: Session = (session ??= { passportUploadKey: newKey(), slipUploadKey: newKey() });

    try {
      const refusal = store.entry()?.refusal();
      if (refusal) throw new Refused(refusal, 'entryRefused');

      // 1. the uploads (today's route, raw bytes)
      if (!s.passportKey) {
        const file = store.passport();
        if (!file) throw new Refused(t('noPassport'), 'noPassport');
        s.passportKey = await upload(file, s.passportUploadKey);
      }
      const slipFile = store.slip();
      if (slipFile && !s.slipKey) {
        s.slipKey = await upload(slipFile, s.slipUploadKey);
      }

      // 2. a quote that has not expired (D-5), then the create, made once
      if (!s.create) {
        let q = store.quote();
        if (!q || expired(q.expiresAt)) q = await store.requote();
        if (!q) throw new Refused(store.quoteError()?.message ?? t('noQuote'), 'noQuote');
        s.create = { path: routes.transactions, body: createBody(q.id, s), key: newKey() };
      }
      if (!s.tx) s.tx = await send(s.create);

      // 3. the server's state decides the next call
      for (;;) {
        const tx: NonNullable<Session['tx']> = s.tx;
        sink.setTransactionState(tx.status);
        switch (tx.status) {
          case 'DRAFT': {
            const body: TransactionConfirmInput = { id: tx.id };
            s.confirm ??= { path: `${routes.transactions}/${tx.id}/confirm`, body, key: newKey() };
            s.tx = await send(s.confirm);
            break;
          }
          case 'CONFIRMED': {
            s.complete ??= { path: `${routes.transactions}/${tx.id}/complete`, body: { id: tx.id }, key: newKey() };
            s.tx = await send(s.complete);
            break;
          }
          case 'COMPLETED': {
            stopWatch();
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
            session = undefined;
            sink.setSaveError(typeof reason === 'string' && reason !== '' ? reason : t('cancelled'));
            sink.setOutcome('refused');
            return 'refused';
          }
        }
      }
    } catch (e) {
      if (e instanceof Refused || (e instanceof ApiError && e.status >= 400 && e.status < 500 && e.code !== 'httpError')) {
        // a refusal: the next submit is a new exchange
        reset();
        sink.setSaveError(e.message);
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
