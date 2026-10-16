import { createSignal, type Accessor } from 'solid-js';
import { ApiClient, ApiError } from '../../../../shared/api/client';
import type { DeskRefundRow, DeskRefunds } from '../../../../shared/api/generated/desk_refunds';
import type { ReceiptDocumentViewData } from '../../../../shared/api/generated/receipt_document';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import { ReceiptFrame } from '../../../../shared/receipt/ReceiptFrame';
import * as texts from './i18n';
import { createPoster, type Poster } from './post';

const t = screenT('desk.steps.refunds', texts);

const ERROR_CODES = [
  'reversalWindowClosed',
  'noOpenShift',
  'stockInsufficient',
  'staleVersion',
  'idempotencyConflict',
] as const;

export interface RefundsStore {
  rows: Accessor<DeskRefundRow[]>;
  open: Accessor<boolean>;
  setOpen(v: boolean): void;
  lastPostedDoc: Accessor<string | null>;
  postError: Accessor<string | null>;
  canRetry: Accessor<boolean>;
  confirmingId: Accessor<string | null>;
  startConfirm(id: string): void;
  cancelConfirm(): void;
  submitPost(): Promise<void>;
  retry(): Promise<void>;
  printReceipt(): Promise<void>;
  readRefunds(): Promise<void>;
}

export function createRefundsStore(api: ApiClient): RefundsStore {
  const [rows, setRows] = createSignal<DeskRefundRow[]>([]);
  const [open, setOpen] = createSignal(false);
  const [lastPostedDoc, setLastPostedDoc] = createSignal<string | null>(null);
  const [postError, setPostError] = createSignal<string | null>(null);
  const [canRetry, setCanRetry] = createSignal(false);
  const [confirmingId, setConfirmingId] = createSignal<string | null>(null);

  let currentPoster: Poster | null = null;
  let currentPostingId: string | null = null;

  async function readRefunds(): Promise<void> {
    const res = await api.view<DeskRefunds>('desk_refunds');
    setRows(res.data?.rows ?? []);
  }

  function startConfirm(id: string): void {
    setConfirmingId(id);
  }

  function cancelConfirm(): void {
    setConfirmingId(null);
  }

  async function execute(poster: Poster, id: string): Promise<void> {
    setPostError(null);
    setCanRetry(false);
    let doc: string;
    try {
      doc = await poster.post();
    } catch (err) {
      if (err instanceof ApiError && (ERROR_CODES as readonly string[]).includes(err.code)) {
        const code = err.code as (typeof ERROR_CODES)[number];
        setPostError(t(`error.${code}`));
        setCanRetry(false);
        currentPoster = null;
        currentPostingId = null;
      } else if (!(err instanceof ApiError) || err.status >= 500) {
        setPostError(t('error.unknown'));
        setCanRetry(true);
      } else {
        setPostError(wordedError(err));
        setCanRetry(false);
        currentPoster = null;
        currentPostingId = null;
      }
      return;
    }

    currentPoster = null;
    currentPostingId = null;
    setLastPostedDoc(doc);
    setRows(rows().filter((r) => r.reversal.id !== id));
    try {
      await readRefunds();
    } catch (err) {
      setPostError(wordedError(err));
      setCanRetry(false);
    }
  }

  async function submitPost(): Promise<void> {
    const id = confirmingId();
    if (!id) return;
    setConfirmingId(null);
    currentPoster = createPoster(api, id);
    currentPostingId = id;
    await execute(currentPoster, id);
  }

  async function retry(): Promise<void> {
    if (!currentPoster || !currentPostingId) return;
    await execute(currentPoster, currentPostingId);
  }

  async function printReceipt(): Promise<void> {
    const doc = lastPostedDoc();
    if (!doc) return;
    const res = await api.view<ReceiptDocumentViewData>('receipt_document', {
      filter: { number: doc },
    });
    const html = res.data?.receipt?.html;
    if (html) {
      const doPrint = ReceiptFrame.print;
      await doPrint(html);
    }
  }

  return {
    rows,
    open,
    setOpen,
    lastPostedDoc,
    postError,
    canRetry,
    confirmingId,
    startConfirm,
    cancelConfirm,
    submitPost,
    retry,
    printReceipt,
    readRefunds,
  };
}
