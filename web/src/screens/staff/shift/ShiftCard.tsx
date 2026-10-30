import { createEffect, createSignal, onCleanup, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { checkInteger } from '../../../shared/ui/bounded';
import { yieldQueueCard } from '../../../shared/ui/queue-card-yield';
import { onFreshNotice } from '../../../shared/ui/notice-bell/notice-feed';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  CountSubmitInput,
  HandoverStartInput,
  CashRecordInput,
  CashRecordOutput,
  CountVerifyOutput,
  CashLineInput,
} from '../../../shared/api/generated/cash';
import type { CashCountRow, CashCountsFilter } from '../../../shared/api/generated/cash_counts';
import type { CountRejectInput, CountRejected } from '../../../shared/api/generated/staff_corrections';
import type { HandoverPendingRow } from '../../../shared/api/generated/handovers_pending';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { HandoverReceiverRow } from '../../../shared/api/generated/handover_receivers';
import type { MyShift } from '../../../shared/api/generated/my_shift';
import { CurrencyCount } from '../../../shared/ui/currency-count/CurrencyCount';
import { formatFaceValue } from '../../../shared/ui/currency-count/face-value';
import type { StaffCtx } from '../cards';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { th, en } from './i18n';
import './shift.css';

export interface ShiftCardProps {
  ctx: StaffCtx;
}

const COUNT_DAY_FMT = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Bangkok',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
});

const COUNT_TIME_FMT = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

function formatCountTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const [, mm, dd] = COUNT_DAY_FMT.format(d).split('-');
  const time = COUNT_TIME_FMT.format(d);
  return `${dd}/${mm} ${time}`;
}

const COUNT_KINDS = ['OPEN', 'CLOSE', 'HANDOVER_OUT', 'HANDOVER_IN'] as const;
type CountKind = (typeof COUNT_KINDS)[number];
const isCountKind = (k: string): k is CountKind => (COUNT_KINDS as readonly string[]).includes(k);

export function ShiftCard(props: ShiftCardProps): JSX.Element {
  const t = screenT('staff.shift', { th, en });
  const kindLabel = (kind: string): string => (isCountKind(kind) ? t(kind) : kind);
  const api = new ApiClient();
  const commandMap = new Map<string, { send(body: unknown): Promise<unknown> }>();

  function getCmd<B, R>(key: string, method: string, path: string): { send(body: B): Promise<R> } {
    let c = commandMap.get(key);
    if (!c) {
      c = command<B, R>(api, method, path);
      commandMap.set(key, c as { send(body: unknown): Promise<unknown> });
    }
    return c as { send(body: B): Promise<R> };
  }

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let errorFrom: string | null = null;
  // #2409: one write at a time; a second click or Enter while one is in flight does nothing.
  const [busy, setBusy] = createSignal<boolean>(false);

  const [cashCounts, setCashCounts] = createSignal<CashCountRow[]>([]);
  const [pendingHandovers, setPendingHandovers] = createSignal<HandoverPendingRow[]>([]);
  const [denominations, setDenominations] = createSignal<DenominationRow[]>([]);
  const [handoverReceivers, setHandoverReceivers] = createSignal<HandoverReceiverRow[]>([]);

  const [countKind, setCountKind] = createSignal<'OPEN' | 'CLOSE' | null>(null);
  const [myShift, setMyShift] = createSignal<MyShift | null>(null);
  const [countQuantities, setCountQuantities] = createSignal<Record<number, string>>({});
  const [countLineErrors, setCountLineErrors] = createSignal<Record<number, string>>({});
  const [showHandover, setShowHandover] = createSignal<boolean>(false);
  const [selectedReceiver, setSelectedReceiver] = createSignal<string>('');
  const [handoverQuantities, setHandoverQuantities] = createSignal<Record<number, string>>({});
  const [handoverLineErrors, setHandoverLineErrors] = createSignal<Record<number, string>>({});
  const [rejectingId, setRejectingId] = createSignal<string | null>(null);
  const [rejectReason, setRejectReason] = createSignal<string>('');
  const [rejectReasonError, setRejectReasonError] = createSignal<string | null>(null);
  const [rejectRowErrors, setRejectRowErrors] = createSignal<Record<string, string>>({});

  createEffect(() => {
    if (countKind() !== null || showHandover() || rejectingId() !== null) {
      const release = yieldQueueCard();
      onCleanup(release);
    }
  });

  const canOperate = () => can(props.ctx.me, 'ShiftOperate');
  const canCertify = () => can(props.ctx.me, 'ShiftCloseCertify');
  const isClockedIn = () => !!props.ctx.floor?.myClockIn;
  const hasOpenShift = () => !!props.ctx.floor?.myShift;

  const loadCashCounts = async () => {
    if (!canCertify() && !canOperate()) return;
    try {
      const res = await api.view<CashCountRow[] | { rows: CashCountRow[] }, Partial<CashCountsFilter>>('cash_counts', {
        branch: props.ctx.me.branchId,
        filter: { status: 'WAITING' },
      });
      const data = res.data;
      if (Array.isArray(data)) {
        setCashCounts(data);
      } else if (data && Array.isArray((data as { rows: CashCountRow[] }).rows)) {
        setCashCounts((data as { rows: CashCountRow[] }).rows);
      }
      if (errorFrom === 'loadCashCounts') {
        errorFrom = null;
        setError(null);
      }
    } catch (err: unknown) {
      errorFrom = 'loadCashCounts';
      setError(wordedError(err));
    }
  };

  const loadPendingHandovers = async () => {
    if (!canOperate()) return;
    try {
      const res = await api.view<HandoverPendingRow[] | { rows: HandoverPendingRow[] }>('handovers_pending');
      const data = res.data;
      if (Array.isArray(data)) {
        setPendingHandovers(data);
      } else if (data && Array.isArray((data as { rows: HandoverPendingRow[] }).rows)) {
        setPendingHandovers((data as { rows: HandoverPendingRow[] }).rows);
      }
      if (errorFrom === 'loadPendingHandovers') {
        errorFrom = null;
        setError(null);
      }
    } catch (err: unknown) {
      errorFrom = 'loadPendingHandovers';
      setError(wordedError(err));
    }
  };

  const loadDenominations = async () => {
    if (!canOperate()) return;
    try {
      const res = await api.view<DenominationRow[] | { rows: DenominationRow[] }>('denominations');
      const data = res.data;
      if (Array.isArray(data)) {
        setDenominations(data);
      } else if (data && Array.isArray((data as { rows: DenominationRow[] }).rows)) {
        setDenominations((data as { rows: DenominationRow[] }).rows);
      }
      if (errorFrom === 'loadDenominations') {
        errorFrom = null;
        setError(null);
      }
    } catch (err: unknown) {
      errorFrom = 'loadDenominations';
      setError(wordedError(err));
    }
  };

  const loadHandoverReceivers = async () => {
    if (!canOperate()) return;
    try {
      const res = await api.view<HandoverReceiverRow[] | { rows: HandoverReceiverRow[] }>('handover_receivers');
      const data = res.data;
      if (Array.isArray(data)) {
        setHandoverReceivers(data);
      } else if (data && Array.isArray((data as { rows: HandoverReceiverRow[] }).rows)) {
        setHandoverReceivers((data as { rows: HandoverReceiverRow[] }).rows);
      }
      if (errorFrom === 'loadHandoverReceivers') {
        errorFrom = null;
        setError(null);
      }
    } catch (err: unknown) {
      errorFrom = 'loadHandoverReceivers';
      setError(wordedError(err));
    }
  };

  onMount(() => {
    if (canCertify() || canOperate()) {
      void loadCashCounts();
    }
    if (canOperate()) {
      void loadPendingHandovers();
      void loadDenominations();
    }
    const stop = onFreshNotice('cash_count', () => {
      void loadCashCounts();
    });
    onCleanup(stop);
  });

  const loadMyShift = async () => {
    try {
      const res = await api.view<MyShift | null>('my_shift');
      setMyShift(res.data ?? null);
    } catch {
      setMyShift(null);
    }
  };

  const handleOpenCount = (kind: 'OPEN' | 'CLOSE') => {
    setCountKind(kind);
    setCountLineErrors({});
    setShowHandover(false);
    setMyShift(null);
    if (kind === 'CLOSE') {
      void loadMyShift();
    }
    if (denominations().length === 0) {
      void loadDenominations();
    }
  };

  const countLocked = (): string[] => {
    const codes = ['THB'];
    if (countKind() === 'CLOSE') {
      for (const b of myShift()?.balances ?? []) {
        if (/[1-9]/.test(b.expected) && !codes.includes(b.code)) {
          codes.push(b.code);
        }
      }
    }
    return codes;
  };

  const countExpected = (): Record<string, string> | undefined => {
    if (countKind() !== 'CLOSE') return undefined;
    const shift = myShift();
    if (!shift) return undefined;
    const out: Record<string, string> = {};
    for (const b of shift.balances) out[b.code] = b.expected;
    return out;
  };

  const handleOpenHandover = () => {
    setShowHandover(true);
    setHandoverLineErrors({});
    setCountKind(null);
    setSelectedReceiver('');
    void loadHandoverReceivers();
    if (denominations().length === 0) {
      void loadDenominations();
    }
  };

  const handleCountSubmit = async (e: Event) => {
    e.preventDefault();
    const form = e.currentTarget as HTMLFormElement;
    const kind = countKind();
    if (busy() || !kind || denominations().length === 0) return;

    const inputs = form.querySelectorAll<HTMLInputElement>('input[data-denomination-id]');
    const lines: CashLineInput[] = [];
    const nextQuantities: Record<number, string> = {};
    const nextErrors: Record<number, string> = {};
    const labels: Record<string, string> = {};
    const lineDenomIds: number[] = [];

    inputs.forEach((input) => {
      const idStr = input.getAttribute('data-denomination-id');
      if (!idStr) return;
      const d = denominations().find((x) => String(x.id) === idStr);
      const id = d ? d.id : +idStr;
      const qty = input.value.trim();
      nextQuantities[id] = input.value;
      if (qty !== '' && d) {
        const check = checkInteger(qty, 0, 2147483647);
        if (!check.ok) {
          nextErrors[d.id] = check.error === 'outOfRange' ? t('qtyTooLarge') : t('qtyWholeMin0');
          return;
        }
        const idx = lines.length;
        labels[`lines[${idx}].qty`] = `${d.currency} ${d.faceValue}`;
        lineDenomIds.push(d.id);
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qty: check.text,
        });
      }
    });

    setCountQuantities(nextQuantities);
    if (Object.keys(nextErrors).length > 0) {
      setCountLineErrors(nextErrors);
      return;
    }
    setCountLineErrors({});

    const body: CountSubmitInput = {
      branch: props.ctx.me.branchId,
      kind,
      shiftId: kind === 'CLOSE' ? (props.ctx.floor?.myShift?.id ?? null) : null,
      lines,
      note: null,
    };

    const cmd = getCmd<CountSubmitInput, { id: string }>('count-submit', 'POST', '/api/cash-counts');
    setBusy(true);
    try {
      await cmd.send(body);
      setNotice(t('saved'));
      errorFrom = null;
      setError(null);
      setCountKind(null);
      setCountQuantities({});
      setCountLineErrors({});
      await props.ctx.refresh();
      await loadCashCounts();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      const fe = fieldError(err, labels);
      const match = fe.field?.match(/^lines\[(\d+)\]\.qty$/);
      const denomId = match ? lineDenomIds[+match[1]!] : undefined;
      if (denomId !== undefined) {
        setCountLineErrors({ [denomId]: fe.message });
      } else {
        setError(fe.message);
      }
    } finally {
      setBusy(false);
    }
  };

  const handleHandoverSubmit = async (e: Event) => {
    e.preventDefault();
    const form = e.currentTarget as HTMLFormElement;
    const receiver = selectedReceiver();
    if (busy() || !receiver || availableReceivers().length === 0) return;

    const inputs = form.querySelectorAll<HTMLInputElement>('input[data-denomination-id]');
    const lines: CashLineInput[] = [];
    const nextQuantities: Record<number, string> = {};
    const nextErrors: Record<number, string> = {};
    const labels: Record<string, string> = { receiverId: t('receiver') };
    const lineDenomIds: number[] = [];

    inputs.forEach((input) => {
      const idStr = input.getAttribute('data-denomination-id');
      if (!idStr) return;
      const d = denominations().find((x) => String(x.id) === idStr);
      const id = d ? d.id : +idStr;
      const qty = input.value.trim();
      nextQuantities[id] = input.value;
      if (qty !== '' && d) {
        const check = checkInteger(qty, 0, 2147483647);
        if (!check.ok) {
          nextErrors[d.id] = check.error === 'outOfRange' ? t('qtyTooLarge') : t('qtyWholeMin0');
          return;
        }
        const idx = lines.length;
        labels[`lines[${idx}].qty`] = `${d.currency} ${d.faceValue}`;
        lineDenomIds.push(d.id);
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qty: check.text,
        });
      }
    });

    setHandoverQuantities(nextQuantities);
    if (Object.keys(nextErrors).length > 0) {
      setHandoverLineErrors(nextErrors);
      return;
    }
    setHandoverLineErrors({});

    const body: HandoverStartInput = {
      branch: props.ctx.me.branchId,
      fromShiftId: props.ctx.floor?.myShift?.id ?? '',
      receiverId: receiver,
      lines,
      note: null,
    };

    const cmd = getCmd<HandoverStartInput, { id: string }>('handover-start', 'POST', '/api/handovers');
    setBusy(true);
    try {
      await cmd.send(body);
      setNotice(t('saved'));
      errorFrom = null;
      setError(null);
      setShowHandover(false);
      setSelectedReceiver('');
      setHandoverQuantities({});
      setHandoverLineErrors({});
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      const fe = fieldError(err, labels);
      const match = fe.field?.match(/^lines\[(\d+)\]\.qty$/);
      const denomId = match ? lineDenomIds[+match[1]!] : undefined;
      if (denomId !== undefined) {
        setHandoverLineErrors({ [denomId]: fe.message });
      } else {
        setError(fe.message);
      }
    } finally {
      setBusy(false);
    }
  };

  const handleVerifyCount = async (id: string) => {
    if (busy()) return;
    setBusy(true);
    const cmd = getCmd<CashRecordInput, CountVerifyOutput>(
      'count-verify-' + id,
      'POST',
      '/api/cash-counts/' + id + '/verify'
    );
    try {
      const answer = await cmd.send({ id, branch: props.ctx.me.branchId });
      if (answer?.status === 'AWAITING_FINANCE') {
        setNotice(
          `${t('countAwaitingFinance')} (${t('approvalRequest')} ${answer.approvalRequestId ?? ''})`
        );
      } else {
        setNotice(t('saved'));
      }
      errorFrom = null;
      setError(null);
      await props.ctx.refresh();
      await loadCashCounts();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const handleAcceptHandover = async (id: string) => {
    if (busy()) return;
    setBusy(true);
    const cmd = getCmd<CashRecordInput, CashRecordOutput>(
      'handover-accept-' + id,
      'POST',
      '/api/handovers/' + id + '/accept'
    );
    try {
      const answer = await cmd.send({ id, branch: props.ctx.me.branchId });
      if (answer?.status === 'AWAITING_FINANCE') {
        setNotice(t('handoverAwaitingFinance'));
      } else {
        setNotice(t('saved'));
      }
      errorFrom = null;
      setError(null);
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const handleRejectHandover = async (id: string) => {
    if (busy()) return;
    setBusy(true);
    const cmd = getCmd<CashRecordInput, CashRecordOutput>(
      'handover-reject-' + id,
      'POST',
      '/api/handovers/' + id + '/reject'
    );
    try {
      await cmd.send({ id, branch: props.ctx.me.branchId });
      setNotice(t('saved'));
      errorFrom = null;
      setError(null);
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const handleOpenRejectCount = (id: string) => {
    if (busy()) return;
    setRejectingId(id);
    setRejectReason('');
    setRejectReasonError(null);
    setRejectRowErrors((prev) => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
  };

  const handleCancelRejectCount = () => {
    if (busy()) return;
    setRejectingId(null);
    setRejectReason('');
    setRejectReasonError(null);
  };

  const handleConfirmRejectCount = async (id: string) => {
    if (busy()) return;
    const trimmed = rejectReason().trim();
    if (!trimmed) {
      setRejectReasonError(t('reasonRequired'));
      return;
    }
    if (trimmed.length > 500) {
      setRejectReasonError(t('reasonTooLong'));
      return;
    }
    setRejectReasonError(null);
    setBusy(true);
    const cmd = getCmd<Pick<CountRejectInput, 'reason'>, CountRejected>(
      'count-reject-' + id,
      'POST',
      '/api/cash-counts/' + id + '/reject'
    );
    try {
      await cmd.send({ reason: trimmed });
      setNotice(t('saved'));
      errorFrom = null;
      setError(null);
      setRejectingId(null);
      setRejectReason('');
      setRejectReasonError(null);
      setRejectRowErrors((prev) => {
        const next = { ...prev };
        delete next[id];
        return next;
      });
      await props.ctx.refresh();
      await loadCashCounts();
    } catch (err: unknown) {
      setNotice(null);
      errorFrom = null;
      const fe = fieldError(err, { reason: t('reason') });
      if (err instanceof ApiError && !isServerFailure(err)) {
        if (err.code === 'reasonRequired' || err.code === 'reasonTooLong') {
          setRejectReasonError(t(err.code));
          return;
        }
        if (fe.field === 'reason') {
          setRejectReasonError(fe.message);
          return;
        }
        const rowMsg =
          err.code === 'notWaiting' ||
          err.code === 'sameCounter' ||
          err.code === 'onlyThroughHandover'
            ? t(err.code)
            : fe.message;
        setRejectRowErrors((prev) => ({ ...prev, [id]: rowMsg }));
        if (err.code === 'notWaiting') {
          await loadCashCounts();
        }
      } else {
        setRejectRowErrors((prev) => ({ ...prev, [id]: fe.message }));
      }
    } finally {
      setBusy(false);
    }
  };

  const isWaitingCount = (c: CashCountRow) => c.status === 'WAITING' && c.verifiedBy === null;

  const countsToVerify = () => {
    if (!canCertify()) return [];
    return cashCounts().filter(
      (c) =>
        isWaitingCount(c) &&
        (c.kind === 'OPEN' || c.kind === 'CLOSE') &&
        c.countedBy !== props.ctx.me.userId
    );
  };

  const waitingOpenCount = () => {
    return cashCounts().find(
      (c) =>
        c.kind === 'OPEN' &&
        c.countedBy === props.ctx.me.userId &&
        isWaitingCount(c)
    );
  };

  const waitingCloseCount = () => {
    const shiftId = props.ctx.floor?.myShift?.id;
    if (!shiftId) return undefined;
    return cashCounts().find(
      (c) =>
        c.kind === 'CLOSE' &&
        c.countedBy === props.ctx.me.userId &&
        isWaitingCount(c) &&
        c.shiftId === shiftId
    );
  };

  const myCloseWaiting = () => waitingCloseCount() !== undefined;

  const handoversToMe = () => {
    if (!canOperate()) return [];
    return pendingHandovers().filter(
      (h) => h.receiverId === props.ctx.me.userId && h.status === 'PENDING'
    );
  };

  const availableReceivers = () => {
    return handoverReceivers().filter((r) => r.userId !== props.ctx.me.userId);
  };

  return (
    <div class="xc-shift-card">
      <div class="xc-shift-header">
        <h2 class="xc-shift-title">{t('title')}</h2>
      </div>

      <Show when={notice()}>
        <div data-testid="shift-notice" class="xc-shift-notice">
          {notice()}
        </div>
      </Show>

      <Show when={error()}>
        <div data-testid="shift-error" class="xc-shift-error">
          {error()}
        </div>
      </Show>

      <Show when={canOperate() && isClockedIn()}>
        <div class="xc-shift-actions">
          <Show when={!hasOpenShift()}>
            <Show
              when={!waitingOpenCount()}
              fallback={
                <div role="status" class="xc-shift-close-waiting" data-testid="shift-open-waiting">
                  {t('openWaiting')} ({formatCountTime(waitingOpenCount()!.countedAt)})
                </div>
              }
            >
              <button
                type="button"
                data-testid="shift-open-count"
                class="xc-shift-action-btn"
                onClick={() => handleOpenCount('OPEN')}
              >
                {t('openCount')}
              </button>
            </Show>
          </Show>
          <Show when={hasOpenShift()}>
            <Show
              when={!myCloseWaiting()}
              fallback={
                <div role="status" class="xc-shift-close-waiting" data-testid="shift-close-waiting">
                  {t('closeWaiting')} ({formatCountTime(waitingCloseCount()!.countedAt)})
                </div>
              }
            >
              <button
                type="button"
                data-testid="shift-close-count"
                class="xc-shift-action-btn"
                onClick={() => handleOpenCount('CLOSE')}
              >
                {t('closeCount')}
              </button>
            </Show>
            <button
              type="button"
              data-testid="shift-handover"
              class="xc-shift-action-btn"
              onClick={handleOpenHandover}
            >
              {t('handover')}
            </button>
          </Show>
        </div>
      </Show>

      <Show when={countKind() !== null}>
        <form
          data-testid="shift-count-form"
          class="xc-shift-count-form"
          onSubmit={handleCountSubmit}
        >
          <CurrencyCount
            denominations={denominations()}
            locked={countLocked()}
            expected={countExpected()}
            quantities={countQuantities()}
            setQuantity={(id, v) => setCountQuantities((prev) => ({ ...prev, [id]: v }))}
            lineErrors={countLineErrors()}
          />
          <div class="xc-shift-form-actions">
            <button
              type="submit"
              data-testid="shift-count-save"
              class="xc-shift-save-btn"
              disabled={busy() || denominations().length === 0}
            >
              {t('save')}
            </button>
            <button
              type="button"
              class="xc-shift-cancel-btn"
              onClick={() => {
                setCountKind(null);
                setCountQuantities({});
                setCountLineErrors({});
              }}
            >
              {t('cancel')}
            </button>
          </div>
        </form>
      </Show>

      <Show when={showHandover()}>
        <form
          data-testid="shift-handover-form"
          class="xc-shift-handover-form"
          onSubmit={handleHandoverSubmit}
        >
          <div class="xc-shift-form-group">
            <label class="xc-shift-form-label">{t('receiver')}</label>
            <select
              data-testid="shift-receiver"
              class="xc-shift-select"
              value={selectedReceiver()}
              onInput={(e) => setSelectedReceiver(e.currentTarget.value)}
              onChange={(e) => setSelectedReceiver(e.currentTarget.value)}
            >
              <option value="">{t('selectReceiver')}</option>
              <For each={availableReceivers()}>
                {(r) => <option value={r.userId}>{r.name}</option>}
              </For>
            </select>
          </div>
          <div class="xc-shift-denoms">
            <For each={denominations()}>
              {(d) => (
                <div data-denomination-row data-face-value={d.faceValue} class="xc-shift-denom-row">
                  <span class="xc-shift-denom-label">
                    <span class="xc-shift-denom-currency">{d.currency}</span>{' '}
                    <span class="xc-shift-denom-val">{formatFaceValue(d.faceValue)}</span>
                  </span>
                  <input
                    data-denomination-id={d.id}
                    class="xc-shift-denom-input"
                    type="text"
                    inputMode="numeric"
                    value={handoverQuantities()[d.id] ?? ''}
                    onInput={(e) => {
                      const v = e.currentTarget.value;
                      setHandoverQuantities((prev) => ({ ...prev, [d.id]: v }));
                    }}
                    onChange={(e) => {
                      const v = e.currentTarget.value;
                      setHandoverQuantities((prev) => ({ ...prev, [d.id]: v }));
                    }}
                  />
                  <Show when={handoverLineErrors()[d.id]}>
                    {(msg) => <span data-field-error={`lines.${d.id}.qty`}>{msg()}</span>}
                  </Show>
                </div>
              )}
            </For>
          </div>
          <div class="xc-shift-form-actions">
            <button
              type="submit"
              data-testid="shift-handover-save"
              class="xc-shift-save-btn"
              disabled={busy() || availableReceivers().length === 0 || !selectedReceiver()}
            >
              {t('save')}
            </button>
            <button
              type="button"
              class="xc-shift-cancel-btn"
              onClick={() => {
                setShowHandover(false);
                setSelectedReceiver('');
                setHandoverQuantities({});
                setHandoverLineErrors({});
              }}
            >
              {t('cancel')}
            </button>
          </div>
        </form>
      </Show>

      <Show when={canCertify() && countsToVerify().length > 0}>
        <div class="xc-shift-section">
          <h3 class="xc-shift-section-title">{t('countsToVerify')}</h3>
          <div class="xc-shift-list">
            <For each={countsToVerify()}>
              {(c) => (
                <div data-count-id={c.id} data-kind={c.kind} class="xc-shift-count-item">
                  <div class="xc-shift-item-info">
                    <span class="xc-shift-item-title">{kindLabel(c.kind)}</span>
                    <span class="xc-shift-item-sub">
                      {formatCountTime(c.countedAt)}
                      {' · '}
                      <span title={c.id}>#{c.id.slice(0, 8)}</span>
                      <For each={c.totals}>
                        {(tot) => <> · {tot.currency} {tot.counted}</>}
                      </For>
                    </span>
                    <Show when={rejectRowErrors()[c.id]}>
                      {(msg) => (
                        <span data-testid="shift-count-error" class="xc-shift-count-error">
                          {msg()}
                        </span>
                      )}
                    </Show>
                  </div>
                  <div class="xc-shift-item-actions">
                    <Show
                      when={rejectingId() === c.id}
                      fallback={
                        <>
                          <button
                            type="button"
                            data-testid="shift-verify"
                            class="xc-shift-btn"
                            disabled={busy()}
                            onClick={() => handleVerifyCount(c.id)}
                          >
                            {t('verify')}
                          </button>
                          <Show when={c.status === 'WAITING'}>
                            <button
                              type="button"
                              data-testid="shift-count-reject"
                              class="xc-shift-btn xc-shift-btn-danger"
                              disabled={busy()}
                              onClick={() => handleOpenRejectCount(c.id)}
                            >
                              {t('reject')}
                            </button>
                          </Show>
                        </>
                      }
                    >
                      <div class="xc-shift-reject-inline">
                        <div class="xc-shift-reject-field">
                          <input
                            type="text"
                            data-testid="shift-count-reject-reason"
                            class="xc-shift-reject-input"
                            maxLength={500}
                            placeholder={t('reason')}
                            aria-label={t('reason')}
                            value={rejectReason()}
                            onInput={(e) => {
                              setRejectReason(e.currentTarget.value);
                              if (rejectReasonError()) setRejectReasonError(null);
                            }}
                            onChange={(e) => setRejectReason(e.currentTarget.value)}
                          />
                          <Show when={rejectReasonError()}>
                            {(msg) => (
                              <span data-field-error="reason" class="xc-shift-field-error">
                                {msg()}
                              </span>
                            )}
                          </Show>
                        </div>
                        <button
                          type="button"
                          data-testid="shift-count-reject-confirm"
                          class="xc-shift-btn xc-shift-btn-danger"
                          disabled={busy()}
                          onClick={() => handleConfirmRejectCount(c.id)}
                        >
                          {t('reject')}
                        </button>
                        <button
                          type="button"
                          data-testid="shift-count-reject-cancel"
                          class="xc-shift-cancel-btn"
                          disabled={busy()}
                          onClick={handleCancelRejectCount}
                        >
                          {t('cancel')}
                        </button>
                      </div>
                    </Show>
                  </div>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>

      <Show when={canOperate() && handoversToMe().length > 0}>
        <div class="xc-shift-section">
          <h3 class="xc-shift-section-title">{t('handoversToMe')}</h3>
          <div class="xc-shift-list">
            <For each={handoversToMe()}>
              {(h) => (
                <div data-handover-id={h.id} class="xc-shift-handover-item">
                  <div class="xc-shift-item-info">
                    <span class="xc-shift-item-title">{h.fromShiftId}</span>
                  </div>
                  <div class="xc-shift-item-actions">
                    <button
                      type="button"
                      data-testid="shift-accept"
                      class="xc-shift-btn"
                      disabled={busy()}
                      onClick={() => handleAcceptHandover(h.id)}
                    >
                      {t('accept')}
                    </button>
                    <button
                      type="button"
                      data-testid="shift-reject"
                      class="xc-shift-btn xc-shift-btn-danger"
                      disabled={busy()}
                      onClick={() => handleRejectHandover(h.id)}
                    >
                      {t('reject')}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}
