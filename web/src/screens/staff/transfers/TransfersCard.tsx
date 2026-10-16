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
import type { TransferBranchRow, TransferBranches } from '../../../shared/api/generated/transfer_branches';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
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

export function TransfersCard(props: TransfersCardProps): JSX.Element {
  const t = screenT('staff.transfers', { th, en });

  const [transfers, setTransfers] = createSignal<CashTransferRow[]>([]);
  const [denominations, setDenominations] = createSignal<DenominationRow[]>([]);
  const [transferBranches, setTransferBranches] = createSignal<TransferBranchRow[]>([]);

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  // Create form state
  const [showCreate, setShowCreate] = createSignal(false);
  const [kind, setKind] = createSignal('BRANCH');
  const [toBranchId, setToBranchId] = createSignal('');
  const [note, setNote] = createSignal('');
  const [quantities, setQuantities] = createSignal<Record<number, string>>({});
  const [submitting, setSubmitting] = createSignal(false);

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
    void loadTransfers();
    void loadDenominations();
    void loadTransferBranches();
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
    setNotice(null);
    setError(null);
  };

  const handleCreateSubmit = async (e: Event) => {
    e.preventDefault();
    if (isCreateSaveDisabled()) return;

    const api = new ApiClient();
    setSubmitting(true);

    try {
      const lines: CashLineInput[] = [];
      for (const d of denominations()) {
        const qStr = quantities()[d.id];
        if (qStr && qStr.trim() !== '') {
          lines.push({
            currency: d.currency,
            denominationId: d.id,
            qty: qStr.trim(),
          });
        }
      }
      const createBody: TransferCreateInput = {
        branch: props.ctx.me.branchId,
        kind: kind(),
        toBranchId: kind() === 'BRANCH' ? toBranchId() || null : null,
        fromShiftId:
          kind() === 'DRAWER_TO_SAFE' ? props.ctx.floor?.myShift?.id ?? null : null,
        toShiftId:
          kind() === 'SAFE_TO_DRAWER' ? props.ctx.floor?.myShift?.id ?? null : null,
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
      setToBranchId('');
      setKind('BRANCH');
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      await loadTransfers();
    } catch (err: unknown) {
      setError(wordedError(err));
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
      setError(wordedError(err));
    } finally {
      setRowBusy(row.id, false);
    }
  };

  const handleOpenReceive = (row: CashTransferRow) => {
    setActiveReceive(row);
    setReceiveQuantities({});
    setError(null);
  };

  const handleReceiveSubmit = async (e: Event) => {
    e.preventDefault();
    const row = activeReceive();
    if (!row || isBusy(row.id)) return;
    setRowBusy(row.id, true);

    const lines: CashReceivedLineInput[] = [];
    for (const d of denominations()) {
      const qStr = receiveQuantities()[d.id];
      if (qStr && qStr.trim() !== '') {
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qtyReceived: qStr.trim(),
        });
      }
    }

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
      setError(null);
      setNotice(t('saved'));
      await props.ctx.refresh();
      await loadTransfers();
    } catch (err: unknown) {
      setError(wordedError(err));
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
      setError(wordedError(err));
    } finally {
      setRowBusy(row.id, false);
    }
  };

  return (
    <div data-card="transfers" class="xc-transfers-card">
      <div class="xc-transfers-header">
        <h2 class="xc-transfers-title">{t('title')}</h2>
        <button
          type="button"
          data-testid="transfer-new"
          onClick={handleOpenCreate}
        >
          {t('newTransfer')}
        </button>
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
              onChange={(e) => setKind(e.currentTarget.value)}
            >
              <For each={KINDS}>
                {(k) => {
                  const isDrawerMove =
                    k === 'DRAWER_TO_SAFE' || k === 'SAFE_TO_DRAWER';
                  const disabled = isDrawerMove && !hasOpenShift();
                  const label = t(('kind_' + k) as any);
                  return (
                    <option value={k} disabled={disabled}>
                      {disabled ? `${label} (${t('noOpenShift')})` : label}
                    </option>
                  );
                }}
              </For>
            </select>
            <Show when={!hasOpenShift()}>
              <span class="xc-transfers-no-shift">{t('noOpenShift')}</span>
            </Show>
          </div>

          <Show when={kind() === 'BRANCH'}>
            <div class="xc-transfers-form-field">
              <label>{t('toBranch')}</label>
              <select
                data-testid="transfer-to-branch"
                value={toBranchId()}
                onChange={(e) => setToBranchId(e.currentTarget.value)}
              >
                <option value="">{t('selectBranch')}</option>
                <For each={transferBranches()}>
                  {(b) => <option value={b.branchId}>{b.name}</option>}
                </For>
              </select>
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

          <div class="xc-transfers-denoms">
            <label>{t('lines')}</label>
            <For each={denominations()}>
              {(d) => (
                <div data-denomination-row class="xc-transfer-denom-row">
                  <span>
                    {d.currency} {d.faceValue}
                  </span>
                  <input
                    type="text"
                    data-denomination-id={String(d.id)}
                    value={quantities()[d.id] ?? ''}
                    onInput={(e) => {
                      const val = e.currentTarget.value;
                      setQuantities((prev) => ({ ...prev, [d.id]: val }));
                    }}
                  />
                </div>
              )}
            </For>
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
                setToBranchId('');
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
            <h3>
              {t('receive')}: {row().id}
            </h3>
            <div class="xc-transfers-denoms">
              <label>{t('lines')}</label>
              <For each={denominations()}>
                {(d) => (
                  <div data-denomination-row class="xc-transfer-denom-row">
                    <span>
                      {d.currency} {d.faceValue}
                    </span>
                    <input
                      type="text"
                      data-denomination-id={String(d.id)}
                      value={receiveQuantities()[d.id] ?? ''}
                      onInput={(e) => {
                        const val = e.currentTarget.value;
                        setReceiveQuantities((prev) => ({ ...prev, [d.id]: val }));
                      }}
                    />
                  </div>
                )}
              </For>
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
          fallback={<p class="xc-transfers-empty">{t('empty')}</p>}
        >
          <table class="xc-transfers-table">
            <tbody>
              <For each={transfers()}>
                {(row) => {
                  const canDispatch = () =>
                    row.status === 'APPROVED' && can(props.ctx.me, 'CashManage');

                  const canReceive = () => {
                    if (row.status !== 'DISPATCHED') return false;
                    if (!can(props.ctx.me, 'CashManage')) return false;
                    return (
                      row.toBranchId === props.ctx.me.branchId ||
                      (!row.toBranchId && row.fromBranchId === props.ctx.me.branchId)
                    );
                  };

                  const canReconcile = () =>
                    row.status === 'RECEIVED' &&
                    can(props.ctx.me, 'FinanceReconcile');

                  return (
                    <tr data-transfer-id={row.id}>
                      <td>{row.id}</td>
                      <td>{t(('kind_' + row.kind) as any) || row.kind}</td>
                      <td>{row.status}</td>
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
