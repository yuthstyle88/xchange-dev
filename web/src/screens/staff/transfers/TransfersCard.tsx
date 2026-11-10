import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  CashLineInput,
  CashReceivedLineInput,
  CashRecordInput,
  CashRecordOutput,
  TransferCreateInput,
  TransferReceiveInput,
  TransferReconcileInput,
} from '../../../shared/api/generated/cash';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { DenominationRow, Denominations } from '../../../shared/api/generated/denominations';
import type { MyShift } from '../../../shared/api/generated/my_shift';
import type { TransferBranchRow, TransferBranches } from '../../../shared/api/generated/transfer_branches';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { checkInteger } from '../../../shared/ui/bounded';
import { CurrencyCount } from '../../../shared/ui/currency-count/CurrencyCount';
import { useBranchCountable } from '../../../shared/ui/currency-count/useBranchCountable';
import type { StaffCtx } from '../cards';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { ReasonDialog } from '../parts/ReasonDialog';
import { th, en } from './i18n';
import './transfers.css';

export interface TransfersCardProps {
  ctx: StaffCtx;
}

const KINDS = [
  'BRANCH',
  'DRAWER_TO_SAFE',
  'SAFE_TO_DRAWER',
  'EXTERNAL_IN',
  'EXTERNAL_OUT',
];

const SHIFT_TIME_FMT = new Intl.DateTimeFormat('en-GB', {
  timeZone: 'Asia/Bangkok',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

function formatShiftTime(iso: string): string {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? iso : SHIFT_TIME_FMT.format(d);
}

interface ParsedDec {
  val: bigint;
  scale: number;
}

function parseFaceValue(raw: string): ParsedDec | null {
  const s = raw.trim();
  if (!/^\d+(\.\d+)?$/.test(s)) return null;
  const dot = s.indexOf('.');
  if (dot === -1) {
    return { val: BigInt(s), scale: 0 };
  }
  const intPart = s.slice(0, dot);
  const fracPart = s.slice(dot + 1).replace(/0+$/, '');
  if (fracPart.length === 0) {
    return { val: BigInt(intPart), scale: 0 };
  }
  return { val: BigInt(intPart + fracPart), scale: fracPart.length };
}

function parseQuantity(raw: string | undefined): bigint {
  if (!raw) return 0n;
  const s = raw.trim();
  if (!/^\d+$/.test(s)) return 0n;
  try {
    return BigInt(s);
  } catch {
    return 0n;
  }
}

function computeRunningTotal(
  items: Array<{ currency: string; faceValue: string; qty: string | undefined }>,
  currencyOrder: string[],
): string {
  const byCurrency = new Map<string, Array<{ faceValue: string; qty: string | undefined }>>();
  for (const item of items) {
    const list = byCurrency.get(item.currency) ?? [];
    list.push(item);
    byCurrency.set(item.currency, list);
  }

  const orderedCurrencies = [...currencyOrder];
  for (const curr of byCurrency.keys()) {
    if (!orderedCurrencies.includes(curr)) {
      orderedCurrencies.push(curr);
    }
  }

  const parts: string[] = [];
  for (const curr of orderedCurrencies) {
    const list = byCurrency.get(curr);
    if (!list) continue;

    let hasQty = false;
    let maxScale = 0;
    const parsedList: Array<{ qty: bigint; dec: ParsedDec }> = [];
    for (const item of list) {
      const q = parseQuantity(item.qty);
      if (q > 0n) {
        hasQty = true;
        const dec = parseFaceValue(item.faceValue);
        if (dec) {
          if (dec.scale > maxScale) maxScale = dec.scale;
          parsedList.push({ qty: q, dec });
        }
      }
    }

    if (!hasQty || parsedList.length === 0) continue;

    let totalScaled = 0n;
    for (const p of parsedList) {
      const scaleDiff = maxScale - p.dec.scale;
      const scaledVal = p.dec.val * 10n ** BigInt(scaleDiff);
      totalScaled += p.qty * scaledVal;
    }

    if (totalScaled === 0n) continue;

    let decStr: string;
    if (maxScale === 0) {
      decStr = totalScaled.toString();
    } else {
      const divisor = 10n ** BigInt(maxScale);
      const intPart = (totalScaled / divisor).toString();
      const fracPart = (totalScaled % divisor).toString().padStart(maxScale, '0').replace(/0+$/, '');
      decStr = fracPart.length > 0 ? `${intPart}.${fracPart}` : intPart;
    }

    parts.push(`${curr} ${formatMoney(decStr)}`);
  }

  return parts.join(' · ');
}

export function TransfersCard(props: TransfersCardProps): JSX.Element {
  const t = screenT('staff.transfers', { th, en });

  const shortId = (id: string): string => `#${id.slice(0, 8)}`;

  const statusLabel = (row: CashTransferRow): string => {
    if (row.status === 'DRAFT') {
      if (row.approvalStatus === 'PENDING') return t('status_PENDING');
      if (row.approvalStatus === 'REJECTED') return t('status_REJECTED');
    }
    switch (row.status) {
      case 'DRAFT':
        return t('status_DRAFT');
      case 'APPROVED':
        return t('status_APPROVED');
      case 'DISPATCHED':
        return t('status_DISPATCHED');
      case 'RECEIVED':
        return t('status_RECEIVED');
      case 'RECONCILED':
        return t('status_RECONCILED');
      default:
        return row.status;
    }
  };

  const api = new ApiClient();
  const countable = useBranchCountable(api, () => props.ctx.me.branchId);

  const [transfers, setTransfers] = createSignal<CashTransferRow[]>([]);
  const [denominations, setDenominations] = createSignal<DenominationRow[]>([]);
  const [transferBranches, setTransferBranches] = createSignal<TransferBranchRow[]>([]);

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  // Create form state
  const [showCreate, setShowCreate] = createSignal(false);
  const [kind, setKind] = createSignal('BRANCH');
  const [toBranchId, setToBranchId] = createSignal('');
  const [toShiftId, setToShiftId] = createSignal('');
  const [note, setNote] = createSignal('');
  const [quantities, setQuantities] = createSignal<Record<number, string>>({});
  const [createLineErrors, setCreateLineErrors] = createSignal<Record<number, string>>({});
  const [toBranchError, setToBranchError] = createSignal<string | null>(null);
  const [toShiftError, setToShiftError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  const currencies = () => {
    const seen = new Set<string>();
    const list: string[] = [];
    for (const d of denominations()) {
      if (!seen.has(d.currency)) {
        seen.add(d.currency);
        list.push(d.currency);
      }
    }
    if (list.includes('THB')) {
      return ['THB', ...list.filter((c) => c !== 'THB')];
    }
    return list;
  };

  const sendTotal = () => {
    const items = denominations().map((d) => ({
      currency: d.currency,
      faceValue: d.faceValue,
      qty: quantities()[d.id],
    }));
    return computeRunningTotal(items, currencies());
  };

  const receiveCurrencies = () => {
    const row = activeReceive();
    if (!row) return [];
    const list: string[] = [];
    for (const line of row.lines) {
      if (!list.includes(line.currency)) {
        list.push(line.currency);
      }
    }
    if (list.includes('THB')) {
      return ['THB', ...list.filter((c) => c !== 'THB')];
    }
    return list;
  };

  const receiveTotal = () => {
    const row = activeReceive();
    if (!row) return '';
    const items = row.lines.map((line) => {
      const d = denominations().find((r) => r.id === line.denominationId);
      return {
        currency: line.currency,
        faceValue: d ? d.faceValue : '0',
        qty: receiveQuantities()[line.denominationId],
      };
    });
    return computeRunningTotal(items, receiveCurrencies());
  };

  // Active command references for create/submit retry
  let currentCreateCmd: ReturnType<typeof command<TransferCreateInput, CashRecordOutput>> | null = null;
  let currentSubmitCmd: ReturnType<typeof command<CashRecordInput, CashRecordOutput>> | null = null;
  let pendingCreatedId: string | null = null;
  let pendingCreateBody: string | null = null;

  // Dispatch command references for retry
  const dispatchCmds = new Map<string, ReturnType<typeof command<CashRecordInput, CashRecordOutput>>>();

  // Receive form state
  const [activeReceive, setActiveReceive] = createSignal<CashTransferRow | null>(null);
  const [receiveQuantities, setReceiveQuantities] = createSignal<Record<number, string>>({});
  const [receiveLineErrors, setReceiveLineErrors] = createSignal<Record<number, string>>({});
  const receiveCmds = new Map<string, ReturnType<typeof command<TransferReceiveInput, CashRecordOutput>>>();

  // Reconcile dialog state
  const [reconcileTarget, setReconcileTarget] = createSignal<CashTransferRow | null>(null);
  const reconcileCmds = new Map<string, ReturnType<typeof command<TransferReconcileInput, CashRecordOutput>>>();

  // Per-row in-flight guard
  const [busyIds, setBusyIds] = createSignal<Set<string>>(new Set());
  const isBusy = (id: string) => busyIds().has(id);
  const setRowBusy = (id: string, busy: boolean) => {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (busy) {
        next.add(id);
      } else {
        next.delete(id);
      }
      return next;
    });
  };

  const hasOpenShift = () => Boolean(props.ctx.floor?.myShift?.id);
  const teller = () => !can(props.ctx.me, 'CashManage');
  const openShifts = () => (!teller() ? (props.ctx.floor?.openShifts ?? []) : []);
  const useShiftPicker = () => !teller() && openShifts().length > 0;
  const canSafeToDrawer = () => useShiftPicker() || hasOpenShift();

  const loadIncoming = async () => {
    try {
      const api = new ApiClient();
      const res = await api.view<MyShift | null>('my_shift');
      const shiftId = res.data?.id;
      const incoming = res.data?.incomingTransfers ?? [];
      setTransfers(shiftId ? incoming.filter((r) => r.toShiftId === shiftId) : []);
    } catch (e: unknown) {
      setError(wordedError(e));
    }
  };

  const loadTransfers = async () => {
    try {
      const api = new ApiClient();
      const res = await api.view<CashTransferRow[]>('cash_transfers', {
        branch: props.ctx.me.branchId,
      });
      if (res.data) {
        setTransfers(res.data);
      }
    } catch (e: unknown) {
      setError(wordedError(e));
    }
  };

  const loadDenominations = async () => {
    try {
      const api = new ApiClient();
      const res = await api.view<Denominations>('denominations');
      if (res.data?.rows) {
        setDenominations(res.data.rows);
      }
    } catch (e: unknown) {
      setError(wordedError(e));
    }
  };

  const loadTransferBranches = async () => {
    try {
      const api = new ApiClient();
      const res = await api.view<TransferBranches>('transfer_branches');
      if (res.data?.rows) {
        const filtered = res.data.rows.filter(
          (b) => b.branchId !== props.ctx.me.branchId
        );
        setTransferBranches(filtered);
      }
    } catch (e: unknown) {
      setError(wordedError(e));
    }
  };

  onMount(() => {
    void loadDenominations();
    if (teller()) {
      void loadIncoming();
    } else {
      void loadTransfers();
      void loadTransferBranches();
    }
  });

  const isCreateSaveDisabled = () => {
    if (submitting()) return true;
    if (kind() === 'BRANCH') {
      if (!toBranchId() || toBranchId() === '') return true;
    }
    return false;
  };

  const handleOpenCreate = () => {
    setShowCreate(true);
    setCreateLineErrors({});
    setToBranchError(null);
    setToShiftError(null);
    setNotice(null);
    setError(null);
  };

  const handleCreateSubmit = async (e: Event) => {
    e.preventDefault();
    if (isCreateSaveDisabled()) return;

    if (kind() === 'SAFE_TO_DRAWER' && useShiftPicker() && !toShiftId()) {
      setToShiftError(t('pickTellerShift'));
      setError(t('pickTellerShift'));
      return;
    }

    const lines: CashLineInput[] = [];
    const nextLineErrors: Record<number, string> = {};
    const labels: Record<string, string> = {
      toBranch: t('toBranch'),
      fromShift: t('kind_DRAWER_TO_SAFE'),
      toShift: t('kind_SAFE_TO_DRAWER'),
    };
    const lineDenomIds: number[] = [];
    for (const d of denominations()) {
      const qStr = (quantities()[d.id] ?? '').trim();
      if (qStr === '') continue;
      const check = checkInteger(qStr, 1, 2147483647);
      if (!check.ok) {
        nextLineErrors[d.id] =
          check.error === 'outOfRange' && !/^-?0+$/.test(qStr) ? t('qtyTooLarge') : t('qtyWholeMin1');
        continue;
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
    if (Object.keys(nextLineErrors).length > 0) {
      setCreateLineErrors(nextLineErrors);
      return;
    }
    setCreateLineErrors({});
    setToBranchError(null);
    setToShiftError(null);

    const api = new ApiClient();
    setSubmitting(true);

    try {
      const createBody: TransferCreateInput = {
        branch: props.ctx.me.branchId,
        kind: kind(),
        toBranchId: kind() === 'BRANCH' ? toBranchId() || null : null,
        fromShiftId:
          kind() === 'DRAWER_TO_SAFE' ? props.ctx.floor?.myShift?.id ?? null : null,
        toShiftId:
          kind() === 'SAFE_TO_DRAWER'
            ? (useShiftPicker() ? toShiftId() || null : props.ctx.floor?.myShift?.id ?? null)
            : null,
        lines,
        note: note(),
      };
      const serializedCreateBody = JSON.stringify(createBody);

      if (pendingCreatedId && pendingCreateBody !== serializedCreateBody) {
        pendingCreatedId = null;
        pendingCreateBody = null;
        currentSubmitCmd = null;
      }

      let createdId = pendingCreatedId;
      if (!createdId) {
        if (!currentCreateCmd) {
          currentCreateCmd = command<TransferCreateInput, CashRecordOutput>(
            api,
            'POST',
            '/api/cash-transfers'
          );
        }

        const res = await currentCreateCmd.send(createBody);
        createdId = res.id;
        pendingCreatedId = createdId;
        pendingCreateBody = serializedCreateBody;
        currentCreateCmd = null;
      }

      if (!currentSubmitCmd) {
        currentSubmitCmd = command<CashRecordInput, CashRecordOutput>(
          api,
          'POST',
          `/api/cash-transfers/${createdId}/submit`
        );
      }
      await currentSubmitCmd.send({
        branch: props.ctx.me.branchId,
        id: createdId,
      });

      // Reset upon success
      currentSubmitCmd = null;
      pendingCreatedId = null;
      pendingCreateBody = null;
      setShowCreate(false);
      setNote('');
      setQuantities({});
      setCreateLineErrors({});
      setToBranchError(null);
      setToShiftError(null);
      setToBranchId('');
      setToShiftId('');
      setKind('BRANCH');
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      await loadTransfers();
    } catch (err: unknown) {
      setNotice(null);
      const fe = fieldError(err, labels);
      const match = fe.field?.match(/^lines\[(\d+)\]\.qty$/);
      const denomId = match ? lineDenomIds[+match[1]!] : undefined;
      if (denomId !== undefined) {
        setCreateLineErrors({ [denomId]: fe.message });
      } else if (fe.field === 'toBranch') {
        setToBranchError(fe.message);
      } else if (fe.field === 'toShift') {
        setToShiftError(fe.message);
      } else {
        setError(fe.message);
      }
    } finally {
      setSubmitting(false);
    }
  };

  const handleDispatch = async (row: CashTransferRow) => {
    if (isBusy(row.id)) return;
    setRowBusy(row.id, true);

    const api = new ApiClient();
    let cmd = dispatchCmds.get(row.id);
    if (!cmd) {
      cmd = command<CashRecordInput, CashRecordOutput>(
        api,
        'POST',
        `/api/cash-transfers/${row.id}/dispatch`
      );
      dispatchCmds.set(row.id, cmd);
    }

    try {
      await cmd.send({ branch: props.ctx.me.branchId, id: row.id });
      dispatchCmds.delete(row.id);
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      await loadTransfers();
    } catch (err: unknown) {
      setNotice(null);
      setError(wordedError(err));
    } finally {
      setRowBusy(row.id, false);
    }
  };

  const denominationText = (id: number): string => {
    const d = denominations().find((row) => row.id === id);
    return d ? `${d.currency} ${d.faceValue}` : `#${id}`;
  };

  const handleOpenReceive = (row: CashTransferRow) => {
    setActiveReceive(row);
    setReceiveQuantities({});
    setReceiveLineErrors({});
    setError(null);
  };

  const handleReceiveSubmit = async (e: Event) => {
    e.preventDefault();
    const row = activeReceive();
    if (!row || isBusy(row.id)) return;

    const lines: CashReceivedLineInput[] = [];
    const nextLineErrors: Record<number, string> = {};
    const labels: Record<string, string> = {};
    for (let i = 0; i < row.lines.length; i++) {
      const l = row.lines[i]!;
      const qStr = (receiveQuantities()[l.denominationId] ?? '').trim();
      const label = denominationText(l.denominationId);
      labels[`lines[${i}].qty`] = label;
      labels[`lines[${i}].qtyReceived`] = label;
      if (qStr === '') {
        nextLineErrors[l.denominationId] = t('countEveryLine');
        continue;
      }
      const check = checkInteger(qStr, 0, 2147483647);
      if (!check.ok) {
        nextLineErrors[l.denominationId] = check.error === 'outOfRange' ? t('qtyTooLarge') : t('qtyWholeMin0');
        continue;
      }
      lines.push({
        currency: l.currency,
        denominationId: l.denominationId,
        qtyReceived: check.text,
      });
    }
    if (Object.keys(nextLineErrors).length > 0) {
      setReceiveLineErrors(nextLineErrors);
      return;
    }
    setReceiveLineErrors({});
    setRowBusy(row.id, true);

    const api = new ApiClient();
    let cmd = receiveCmds.get(row.id);
    if (!cmd) {
      cmd = command<TransferReceiveInput, CashRecordOutput>(
        api,
        'POST',
        `/api/cash-transfers/${row.id}/receive`
      );
      receiveCmds.set(row.id, cmd);
    }

    try {
      await cmd.send({
        branch: props.ctx.me.branchId,
        id: row.id,
        lines,
      });
      receiveCmds.delete(row.id);
      setActiveReceive(null);
      setReceiveQuantities({});
      setReceiveLineErrors({});
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      if (teller()) {
        await loadIncoming();
      } else {
        await loadTransfers();
      }
    } catch (err: unknown) {
      setNotice(null);
      const fe = fieldError(err, labels);
      const match = fe.field?.match(/^lines\[(\d+)\]\.(?:qty|qtyReceived)$/);
      const denomId = match ? row.lines[+match[1]!]?.denominationId : undefined;
      if (denomId !== undefined) {
        setReceiveLineErrors({ [denomId]: fe.message });
      } else {
        setError(fe.message);
      }
    } finally {
      setRowBusy(row.id, false);
    }
  };

  const handleReconcileConfirm = async (reconcileNote: string) => {
    const row = reconcileTarget();
    if (!row || isBusy(row.id)) return;
    setRowBusy(row.id, true);

    let cmd = reconcileCmds.get(row.id);
    if (!cmd) {
      const api = new ApiClient();
      cmd = command<TransferReconcileInput, CashRecordOutput>(
        api,
        'POST',
        `/api/cash-transfers/${row.id}/reconcile`
      );
      reconcileCmds.set(row.id, cmd);
    }

    try {
      await cmd.send({
        branch: props.ctx.me.branchId,
        id: row.id,
        note: reconcileNote,
      });
      reconcileCmds.delete(row.id);
      setReconcileTarget(null);
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      await loadTransfers();
    } catch (err: unknown) {
      setNotice(null);
      setError(wordedError(err));
    } finally {
      setRowBusy(row.id, false);
    }
  };

  return (
    <div class="xc-transfers-card">
      <div class="xc-transfers-header">
        <h2 class="xc-transfers-title">{t(teller() ? 'tellerTitle' : 'title')}</h2>
        <Show when={!teller()}>
          <button
            type="button"
            data-testid="transfer-new"
            onClick={handleOpenCreate}
          >
            {t('newTransfer')}
          </button>
        </Show>
      </div>

      <Show when={notice()}>
        <div data-testid="transfer-notice" class="xc-transfers-notice">
          {notice()}
        </div>
      </Show>

      <Show when={error()}>
        <div data-testid="transfer-error" class="xc-transfers-error">
          {error()}
        </div>
      </Show>

      <Show when={showCreate()}>
        <form
          data-testid="transfer-form"
          class="xc-transfers-form"
          onSubmit={handleCreateSubmit}
        >
          <div class="xc-transfers-form-field">
            <label>{t('kind')}</label>
            <select
              data-testid="transfer-kind"
              value={kind()}
              onChange={(e) => {
                setKind(e.currentTarget.value);
                setToShiftError(null);
              }}
            >
              <For each={KINDS}>
                {(k) => {
                  const disabled = () =>
                    k === 'DRAWER_TO_SAFE'
                      ? !hasOpenShift()
                      : k === 'SAFE_TO_DRAWER'
                        ? !canSafeToDrawer()
                        : false;
                  const label = t(('kind_' + k) as any);
                  return (
                    <option value={k} disabled={disabled()}>
                      {disabled() ? `${label} (${t('noOpenShift')})` : label}
                    </option>
                  );
                }}
              </For>
            </select>
            <Show when={!hasOpenShift() && !useShiftPicker()}>
              <span class="xc-transfers-no-shift">{t('noOpenShift')}</span>
            </Show>
          </div>

          <Show when={kind() === 'BRANCH'}>
            <div class="xc-transfers-form-field">
              <label>{t('toBranch')}</label>
              <select
                data-testid="transfer-to-branch"
                value={toBranchId()}
                onChange={(e) => {
                  setToBranchId(e.currentTarget.value);
                  setToBranchError(null);
                }}
              >
                <option value="">{t('selectBranch')}</option>
                <For each={transferBranches()}>
                  {(b) => <option value={b.branchId}>{b.name}</option>}
                </For>
              </select>
              <Show when={toBranchError()}>
                {(msg) => <span data-field-error="toBranch">{msg()}</span>}
              </Show>
            </div>
          </Show>

          <Show when={kind() === 'SAFE_TO_DRAWER' && useShiftPicker()}>
            <div class="xc-transfers-form-field">
              <label>{t('pickTellerShift')}</label>
              <select
                data-testid="transfer-to-shift"
                value={toShiftId()}
                onChange={(e) => {
                  setToShiftId(e.currentTarget.value);
                  setToShiftError(null);
                  setError(null);
                }}
              >
                <option value="">{t('pickTellerShift')}</option>
                <For each={openShifts()}>
                  {(s) => (
                    <option value={s.shiftId}>
                      {`${s.tellerName} · ${formatShiftTime(s.openedAt)}`}
                    </option>
                  )}
                </For>
              </select>
              <Show when={toShiftError()}>
                {(msg) => <span data-field-error="toShift">{msg()}</span>}
              </Show>
            </div>
          </Show>

          <div class="xc-transfers-form-field">
            <label>{t('note')}</label>
            <input
              type="text"
              data-testid="transfer-note"
              value={note()}
              placeholder={t('notePlaceholder')}
              onInput={(e) => setNote(e.currentTarget.value)}
            />
          </div>

          <CurrencyCount
            denominations={denominations()}
            countable={countable()}
            locked={['THB']}
            quantities={quantities()}
            setQuantity={(id, value) => {
              setQuantities((prev) => ({ ...prev, [id]: value }));
              setCreateLineErrors((prev) => {
                if (!(id in prev)) return prev;
                const next = { ...prev };
                delete next[id];
                return next;
              });
            }}
            lineErrors={createLineErrors()}
          />

          <div data-testid="transfer-total" class="xc-transfers-total">
            {sendTotal()}
          </div>

          <div class="xc-transfers-actions">
            <button
              type="submit"
              data-testid="transfer-save"
              disabled={isCreateSaveDisabled()}
            >
              {t('save')}
            </button>
            <button
              type="button"
              onClick={() => {
                setShowCreate(false);
                pendingCreatedId = null;
                pendingCreateBody = null;
                currentCreateCmd = null;
                currentSubmitCmd = null;
                setNote('');
                setQuantities({});
                setCreateLineErrors({});
                setToBranchError(null);
                setToShiftError(null);
                setToBranchId('');
                setToShiftId('');
                setKind('BRANCH');
                setError(null);
              }}
            >
              {t('cancel')}
            </button>
          </div>
        </form>
      </Show>

      <Show when={activeReceive()}>
        {(row) => (
          <form
            data-testid="transfer-receive-form"
            class="xc-transfers-form"
            onSubmit={handleReceiveSubmit}
          >
            <h3 title={row().id}>
              {t('receive')}: {shortId(row().id)}
            </h3>
            <p class="xc-transfers-note">{t('receiveNote')}</p>
            <div class="xc-transfers-denoms">
              <label>{t('lines')}</label>
              <For each={row().lines}>
                {(line) => (
                  <div data-denomination-row class="xc-transfer-denom-row">
                    <span>
                      {denominationText(line.denominationId)}
                    </span>
                    <input
                      type="text"
                      inputMode="numeric"
                      data-denomination-id={String(line.denominationId)}
                      value={receiveQuantities()[line.denominationId] ?? ''}
                      onInput={(e) => {
                        const val = e.currentTarget.value;
                        setReceiveQuantities((prev) => ({ ...prev, [line.denominationId]: val }));
                      }}
                    />
                    <Show when={receiveLineErrors()[line.denominationId]}>
                      {(msg) => <span data-field-error={`lines.${line.denominationId}.qty`}>{msg()}</span>}
                    </Show>
                  </div>
                )}
              </For>
            </div>
            <div data-testid="transfer-total" class="xc-transfers-total">
              {receiveTotal()}
            </div>
            <div class="xc-transfers-actions">
              <button
                type="submit"
                data-testid="transfer-receive-save"
                disabled={isBusy(row().id)}
              >
                {t('save')}
              </button>
              <button
                type="button"
                onClick={() => {
                  setActiveReceive(null);
                  setReceiveQuantities({});
                  setReceiveLineErrors({});
                }}
              >
                {t('cancel')}
              </button>
            </div>
          </form>
        )}
      </Show>

      <div class="xc-transfers-list">
        <Show
          when={transfers().length > 0}
          fallback={<p class="xc-transfers-empty">{t(teller() ? 'tellerEmpty' : 'empty')}</p>}
        >
          <table class="xc-transfers-table">
            <thead>
              <tr>
                <th>{t('col_id')}</th>
                <th>{t('col_kind')}</th>
                <th>{t('col_status')}</th>
                <th>{t('col_note')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              <For each={transfers()}>
                {(row) => {
                  const canDispatch = () =>
                    !teller() &&
                    row.status === 'APPROVED' &&
                    can(props.ctx.me, 'CashManage');

                  const canReceive = () => {
                    if (row.status !== 'DISPATCHED') return false;
                    if (teller()) return true;
                    if (row.toShiftId) return row.toShiftId === props.ctx.floor?.myShift?.id;
                    if (!can(props.ctx.me, 'CashManage')) return false;
                    return (
                      row.toBranchId === props.ctx.me.branchId ||
                      (!row.toBranchId && row.fromBranchId === props.ctx.me.branchId)
                    );
                  };

                  const canReconcile = () =>
                    !teller() &&
                    row.status === 'RECEIVED' &&
                    can(props.ctx.me, 'FinanceReconcile');

                  return (
                    <tr data-transfer-id={row.id} data-status={row.status}>
                      <td title={row.id}>{shortId(row.id)}</td>
                      <td>{t(('kind_' + row.kind) as any) || row.kind}</td>
                      <td>{statusLabel(row)}</td>
                      <td>{row.note ?? ''}</td>
                      <td>
                        <Show when={canDispatch()}>
                          <button
                            type="button"
                            data-testid="transfer-dispatch"
                            disabled={isBusy(row.id)}
                            onClick={() => handleDispatch(row)}
                          >
                            {t('dispatch')}
                          </button>
                        </Show>
                        <Show when={canReceive()}>
                          <button
                            type="button"
                            data-testid="transfer-receive"
                            disabled={isBusy(row.id)}
                            onClick={() => handleOpenReceive(row)}
                          >
                            {t('receive')}
                          </button>
                        </Show>
                        <Show when={canReconcile()}>
                          <button
                            type="button"
                            data-testid="transfer-reconcile"
                            disabled={isBusy(row.id)}
                            onClick={() => setReconcileTarget(row)}
                          >
                            {t('reconcile')}
                          </button>
                        </Show>
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </Show>
      </div>

      <ReasonDialog
        open={reconcileTarget() !== null}
        title={t('reconcile')}
        onCancel={() => setReconcileTarget(null)}
        onConfirm={handleReconcileConfirm}
      />
    </div>
  );
}
