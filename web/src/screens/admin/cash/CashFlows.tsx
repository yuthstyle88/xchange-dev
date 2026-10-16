// CA-9 #625, stage 1: the admin cash menu's counts and transfers, on CA-8's views and commands (design
// `docs/design/blueprint/cash-shift-transfer.md` §4.4, §4.7). Each section is drawn only when `me.permissions` holds the
// permission its server row checks (`xc-auth/src/domain/policy/areas/cash.rs`), so no role is named here.

import { createEffect, createSignal, For, Show, untrack, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { AdminShiftRow } from '../../../shared/api/generated/admin_shifts';
import type {
  CashLineInput,
  CashReceivedLineInput,
  CashRecordInput,
  CashRecordOutput,
  CountVerifyOutput,
  TransferCreateInput,
  TransferReceiveInput,
  TransferReconcileInput,
} from '../../../shared/api/generated/cash';
import type { CashCountRow, CashCountsFilter } from '../../../shared/api/generated/cash_counts';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { Me } from '../../../shared/api/generated/common';
import type { DenominationRow, Denominations } from '../../../shared/api/generated/denominations';
import type { TransferBranchRow, TransferBranches } from '../../../shared/api/generated/transfer_branches';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Card, CardBody, CardHead } from '../../../shared/ui/Card';
import { closeDialog, openDialog } from '../parts/Dialog';
import { toast } from '../parts/toast';
import * as table from './i18n';

const t = screenT('admin.cash', table);

/** The permissions the server rows check (`areas/cash.rs`). */
const VERIFY = 'ShiftCloseCertify';
const MANAGE = 'CashManage';
const RECONCILE = 'FinanceReconcile';

/** The five transfer kinds (design D:21-23). */
const KINDS = ['BRANCH', 'DRAWER_TO_SAFE', 'SAFE_TO_DRAWER', 'EXTERNAL_IN', 'EXTERNAL_OUT'] as const;

const KIND_TEXT: Record<string, Parameters<typeof t>[0]> = {
  BRANCH: 'kindBranch',
  DRAWER_TO_SAFE: 'kindDrawerToSafe',
  SAFE_TO_DRAWER: 'kindSafeToDrawer',
  EXTERNAL_IN: 'kindExternalIn',
  EXTERNAL_OUT: 'kindExternalOut',
};

const STATUS_TEXT: Record<string, Parameters<typeof t>[0]> = {
  DRAFT: 'transferDraft',
  APPROVED: 'transferApproved',
  DISPATCHED: 'transferDispatched',
  RECEIVED: 'transferReceived',
  RECONCILED: 'transferReconciled',
};

const kindText = (kind: string) => (KIND_TEXT[kind] ? t(KIND_TEXT[kind]!) : kind);
const statusText = (status: string) => (STATUS_TEXT[status] ? t(STATUS_TEXT[status]!) : status);

function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
}

export interface CashFlowsProps {
  branch: () => string;
  shown: () => boolean;
  /** The page's `admin_shifts` rows: the drawer of a move is one of its OPEN shifts. */
  shifts: () => AdminShiftRow[];
}

export function CashFlows(props: CashFlowsProps): JSX.Element {
  const api = new ApiClient();

  const [permissions, setPermissions] = createSignal<readonly string[] | null>(null);
  const can = (permission: string) => (permissions() ?? []).includes(permission);
  const [meError, setMeError] = createSignal<string | null>(null);

  const [counts, setCounts] = createSignal<CashCountRow[]>([]);
  const [transfers, setTransfers] = createSignal<CashTransferRow[]>([]);
  const [denominations, setDenominations] = createSignal<DenominationRow[]>([]);
  const [branches, setBranches] = createSignal<TransferBranchRow[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const [awaiting, setAwaiting] = createSignal<string | null>(null);

  // ── reads ───────────────────────────────────────────────────────────────────

  async function readMe() {
    try {
      const res = await api.view<Me>('me');
      setMeError(null);
      setPermissions(res.data?.permissions ?? []);
    } catch (err: unknown) {
      const text = message(err);
      setRefusal(text);
      setMeError(text);
      setPermissions([]);
    }
  }

  async function readCounts() {
    try {
      setCounts(
        await api.page<CashCountRow, Partial<CashCountsFilter>>('cash_counts', {
          branch: props.branch(),
          filter: { status: 'WAITING' },
        }),
      );
    } catch (err: unknown) {
      setCounts([]);
      setRefusal(message(err));
    }
  }

  async function readTransfers() {
    try {
      setTransfers(await api.page<CashTransferRow>('cash_transfers', { branch: props.branch() }));
    } catch (err: unknown) {
      setTransfers([]);
      setRefusal(message(err));
    }
  }

  async function readForm() {
    const [denoms, dests] = await Promise.allSettled([
      api.view<Denominations>('denominations', { branch: props.branch() }),
      api.view<TransferBranches>('transfer_branches', { branch: props.branch() }),
    ]);
    setDenominations(denoms.status === 'fulfilled' ? (denoms.value.data?.rows ?? []) : []);
    setBranches(dests.status === 'fulfilled' ? (dests.value.data?.rows ?? []) : []);
  }

  let lastBranch: string | undefined;

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    const held = permissions();
    const err = meError();
    untrack(() => {
      if (held === null && err === null) {
        void readMe();
        return;
      }
      if (branch === lastBranch) return;
      lastBranch = branch;
      if (can(VERIFY) || err !== null) void readCounts();
      if (can(MANAGE) || err !== null) {
        void readTransfers();
        void readForm();
      }
    });
  });

  // ── writes: one Idempotency-Key per user action, kept until the server answers ──

  const writes = new Map<string, { send(body: unknown): Promise<unknown>; reset(): void }>();

  function writeFor<B, R>(action: string, path: string): { send(body: B): Promise<R>; reset(): void } {
    let w = writes.get(action);
    if (!w) {
      w = keyedWrite<unknown, unknown>(api, 'POST', path);
      writes.set(action, w);
    }
    return w as { send(body: B): Promise<R>; reset(): void };
  }

  function message(err: unknown): string {
    if (err instanceof ApiError && err.code in table.th) {
      return t(err.code as keyof typeof table.th);
    }
    return wordedError(err);
  }

  async function send<B, R>(action: string, path: string, body: B, done: (answer: R) => void): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setRefusal(null);
    setAwaiting(null);
    try {
      const answer = await writeFor<B, R>(action, path).send(body);
      done(answer);
    } catch (err: unknown) {
      const text = message(err);
      setRefusal(text);
      toast(text);
    } finally {
      setBusy(false);
    }
  }

  function verify(row: CashCountRow) {
    const body: CashRecordInput = { branch: props.branch(), id: row.id };
    void send<CashRecordInput, CountVerifyOutput>(`verify:${row.id}`, `/api/cash-counts/${row.id}/verify`, body, (answer) => {
      if (answer.status === 'AWAITING_FINANCE') {
        setAwaiting(answer.approvalRequestId ?? '');
      } else {
        toast(t('actionSaved'));
      }
      void readCounts();
    });
  }

  function step(row: CashTransferRow, act: 'submit' | 'dispatch') {
    const body: CashRecordInput = { branch: props.branch(), id: row.id };
    void send<CashRecordInput, CashRecordOutput>(`${act}:${row.id}`, `/api/cash-transfers/${row.id}/${act}`, body, () => {
      toast(t('actionSaved'));
      void readTransfers();
    });
  }

  function denominationText(id: number): string {
    const d = denominations().find((row) => row.id === id);
    return d ? `${d.currency} ${d.faceValue}` : `#${id}`;
  }

  function receive(row: CashTransferRow) {
    const [qty, setQty] = createSignal<Record<number, string>>(
      Object.fromEntries(row.lines.map((l) => [l.denominationId, l.qtySent])),
    );
    const onSubmit = (e: Event) => {
      e.preventDefault();
      const lines: CashReceivedLineInput[] = row.lines.map((l) => ({
        currency: l.currency,
        denominationId: l.denominationId,
        qtyReceived: qty()[l.denominationId] ?? '',
      }));
      const body: TransferReceiveInput = { branch: props.branch(), id: row.id, lines };
      void send<TransferReceiveInput, CashRecordOutput>(`receive:${row.id}`, `/api/cash-transfers/${row.id}/receive`, body, () => {
        closeDialog();
        toast(t('actionSaved'));
        void readTransfers();
      });
    };
    openDialog(t('receiveTitle', { id: row.id }), () => (
      <form onSubmit={onSubmit} class="cash-transfer-dialog">
        <p class="dialog-note">{t('receiveNote')}</p>
        <For each={row.lines}>
          {(line) => (
            <div>
              <label for={`receive-${line.denominationId}`}>
                {denominationText(line.denominationId)} · {t('sentQty', { qty: line.qtySent })}
              </label>
              <input
                id={`receive-${line.denominationId}`}
                type="text"
                inputMode="numeric"
                data-denomination={String(line.denominationId)}
                value={qty()[line.denominationId] ?? ''}
                onInput={(e) => {
                  const value = e.currentTarget.value;
                  setQty((prev) => ({ ...prev, [line.denominationId]: value }));
                }}
              />
            </div>
          )}
        </For>
        <div class="dialog-actions">
          <button type="button" class="secondary" onClick={closeDialog}>
            {t('cancel')}
          </button>
          <button type="submit" class="primary" data-action="confirm" disabled={busy()}>
            {t('btnReceive')}
          </button>
        </div>
      </form>
    ));
  }

  function reconcile(row: CashTransferRow) {
    const [note, setNote] = createSignal('');
    const onSubmit = (e: Event) => {
      e.preventDefault();
      const body: TransferReconcileInput = { branch: props.branch(), id: row.id, note: note() };
      void send<TransferReconcileInput, CashRecordOutput>(
        `reconcile:${row.id}`,
        `/api/cash-transfers/${row.id}/reconcile`,
        body,
        () => {
          closeDialog();
          toast(t('actionSaved'));
          void readTransfers();
        },
      );
    };
    openDialog(t('reconcileTitle', { id: row.id }), () => (
      <form onSubmit={onSubmit} class="cash-transfer-dialog">
        <div>
          <label for="reconcile-note">{t('noteLabel')}</label>
          <input
            id="reconcile-note"
            name="note"
            type="text"
            maxLength={200}
            value={note()}
            onInput={(e) => setNote(e.currentTarget.value)}
          />
        </div>
        <div class="dialog-actions">
          <button type="button" class="secondary" onClick={closeDialog}>
            {t('cancel')}
          </button>
          <button type="submit" class="primary" data-action="confirm" disabled={busy()}>
            {t('btnReconcile')}
          </button>
        </div>
      </form>
    ));
  }

  // ── new transfer ────────────────────────────────────────────────────────────

  const [kind, setKind] = createSignal('');
  const [toBranch, setToBranch] = createSignal('');
  const [fromShift, setFromShift] = createSignal('');
  const [toShift, setToShift] = createSignal('');
  const [lineQty, setLineQty] = createSignal<Record<number, string>>({});
  const [note, setNote] = createSignal('');

  const openShifts = () => props.shifts().filter((s) => s.status === 'OPEN');

  function create(e: Event) {
    e.preventDefault();
    const k = kind();
    if (!k) return;
    const typed = lineQty();
    const lines: CashLineInput[] = denominations()
      .filter((d) => (typed[d.id] ?? '').trim() !== '')
      .map((d) => ({ currency: d.currency, denominationId: d.id, qty: typed[d.id]! }));
    const body: TransferCreateInput = {
      branch: props.branch(),
      kind: k,
      toBranchId: k === 'BRANCH' ? toBranch() || null : null,
      fromShiftId: k === 'DRAWER_TO_SAFE' ? fromShift() || null : null,
      toShiftId: k === 'SAFE_TO_DRAWER' ? toShift() || null : null,
      lines,
      note: note(),
    };
    void send<TransferCreateInput, CashRecordOutput>('create', '/api/cash-transfers', body, () => {
      setLineQty({});
      setNote('');
      toast(t('actionSaved'));
      void readTransfers();
    });
  }

  // ── the sections ────────────────────────────────────────────────────────────

  return (
    <div class="cash-flows">
      <Show when={refusal()}>
        <p role="alert" class="error-message">
          {refusal()}
        </p>
      </Show>
      <Show when={awaiting() !== null}>
        <div class="stock-banner" role="status">
          <strong>{t('countAwaitingFinance')}</strong>
          <p>
            {t('approvalRequest')} <span class="ref">{awaiting()}</span>
          </p>
        </div>
      </Show>

      <Show when={can(VERIFY) || Boolean(meError())}>
        <div data-section="counts-waiting">
          <Card>
            <CardHead title={t('countsWaitingTitle')} />
            <Show
              when={counts().length > 0}
              fallback={
                <div class="empty">
                  <strong>{t('emptyCounts')}</strong>
                </div>
              }
            >
              <div class="table-scroll">
                <table class="counts-table">
                  <thead>
                    <tr>
                      <th>{t('headerKind')}</th>
                      <th>{t('headerCountedBy')}</th>
                      <th>{t('headerCountedAt')}</th>
                      <th>{t('headerCurrency')}</th>
                      <th class="num">{t('headerCounted')}</th>
                      <th class="num">{t('headerExpected')}</th>
                      <th class="num">{t('headerDifference')}</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    <For each={counts()}>
                      {(row) => (
                        <For each={row.totals}>
                          {(total, i) => (
                            <tr>
                              <td>{i() === 0 ? row.kind : ''}</td>
                              <td>{i() === 0 ? row.countedBy : ''}</td>
                              <td>{i() === 0 ? when(row.countedAt) : ''}</td>
                              <td>
                                <strong>{total.currency}</strong>
                              </td>
                              <td class="num">{total.counted}</td>
                              <td class="num">{total.expected ?? '—'}</td>
                              <td class="num">{total.difference ?? '—'}</td>
                              <td>
                                <Show when={i() === 0}>
                                  <button
                                    type="button"
                                    class="primary"
                                    data-action="verify"
                                    data-id={row.id}
                                    disabled={busy() || Boolean(meError())}
                                    title={meError() ?? undefined}
                                    onClick={() => verify(row)}
                                  >
                                    {t('btnVerify')}
                                  </button>
                                </Show>
                              </td>
                            </tr>
                          )}
                        </For>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Card>
        </div>
      </Show>

      <Show when={can(MANAGE) || Boolean(meError())}>
        <div data-section="transfers">
          <Card>
            <CardHead title={t('transfersTitle')} />
            <Show
              when={transfers().length > 0}
              fallback={
                <div class="empty">
                  <strong>{t('emptyTransfers')}</strong>
                </div>
              }
            >
              <div class="table-scroll">
                <table class="transfers-table">
                  <thead>
                    <tr>
                      <th>{t('headerCreatedAt')}</th>
                      <th>{t('headerKind')}</th>
                      <th>{t('headerStatus')}</th>
                      <th>{t('headerLines')}</th>
                      <th>{t('headerReason')}</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    <For each={transfers()}>
                      {(row) => (
                        <tr>
                          <td>
                            {when(row.createdAt)} · {row.createdBy}
                          </td>
                          <td>{kindText(row.kind)}</td>
                          <td>
                            <span class={row.status === 'RECONCILED' ? 'pill neutral' : 'pill'}>
                              {statusText(row.status)}
                            </span>
                          </td>
                          <td>
                            <For each={row.lines}>
                              {(line) => (
                                <div>
                                  {denominationText(line.denominationId)} × {line.qtySent}
                                  <Show when={line.qtyReceived !== null}> → {line.qtyReceived}</Show>
                                </div>
                              )}
                            </For>
                          </td>
                          <td>{row.note ?? ''}</td>
                          <td>
                            <Show when={row.status === 'DRAFT'}>
                              <button type="button" class="secondary" data-action="submit" data-id={row.id} disabled={busy() || Boolean(meError())} title={meError() ?? undefined} onClick={() => step(row, 'submit')}>
                                {t('btnSubmit')}
                              </button>
                            </Show>
                            <Show when={row.status === 'APPROVED'}>
                              <button type="button" class="secondary" data-action="dispatch" data-id={row.id} disabled={busy() || Boolean(meError())} title={meError() ?? undefined} onClick={() => step(row, 'dispatch')}>
                                {t('btnDispatch')}
                              </button>
                            </Show>
                            <Show when={row.status === 'DISPATCHED'}>
                              <button type="button" class="secondary" data-action="receive" data-id={row.id} disabled={busy() || Boolean(meError())} title={meError() ?? undefined} onClick={() => receive(row)}>
                                {t('btnReceive')}
                              </button>
                            </Show>
                            <Show when={row.status === 'RECEIVED' && (can(RECONCILE) || Boolean(meError()))}>
                              <button type="button" class="secondary" data-action="reconcile" data-id={row.id} disabled={busy() || Boolean(meError())} title={meError() ?? undefined} onClick={() => reconcile(row)}>
                                {t('btnReconcile')}
                              </button>
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Card>
        </div>

        <div data-section="new-transfer">
          <Card>
            <CardHead title={t('newTransferTitle')} />
            <CardBody>
              <form class="cash-new-transfer" onSubmit={create}>
                <div>
                  <label for="transfer-kind">{t('headerKind')}</label>
                  <select id="transfer-kind" name="kind" value={kind()} onChange={(e) => setKind(e.currentTarget.value)}>
                    <option value="">{t('chooseKind')}</option>
                    <For each={KINDS}>{(k) => <option value={k}>{kindText(k)}</option>}</For>
                  </select>
                </div>

                <Show when={kind() === 'BRANCH'}>
                  <div>
                    <label for="transfer-to-branch">{t('toBranchLabel')}</label>
                    <select
                      id="transfer-to-branch"
                      name="toBranch"
                      value={toBranch()}
                      onChange={(e) => setToBranch(e.currentTarget.value)}
                    >
                      <option value="">{t('chooseBranch')}</option>
                      <For each={branches()}>{(b) => <option value={b.branchId}>{b.name}</option>}</For>
                    </select>
                  </div>
                </Show>

                <Show when={kind() === 'DRAWER_TO_SAFE'}>
                  <div>
                    <label for="transfer-from-shift">{t('fromShiftLabel')}</label>
                    <select
                      id="transfer-from-shift"
                      name="fromShift"
                      value={fromShift()}
                      onChange={(e) => setFromShift(e.currentTarget.value)}
                    >
                      <option value="">{t('chooseShift')}</option>
                      <For each={openShifts()}>{(s) => <option value={s.id}>{`${s.id} · ${s.staff}`}</option>}</For>
                    </select>
                  </div>
                </Show>

                <Show when={kind() === 'SAFE_TO_DRAWER'}>
                  <div>
                    <label for="transfer-to-shift">{t('toShiftLabel')}</label>
                    <select
                      id="transfer-to-shift"
                      name="toShift"
                      value={toShift()}
                      onChange={(e) => setToShift(e.currentTarget.value)}
                    >
                      <option value="">{t('chooseShift')}</option>
                      <For each={openShifts()}>{(s) => <option value={s.id}>{`${s.id} · ${s.staff}`}</option>}</For>
                    </select>
                  </div>
                </Show>

                <div class="transfer-lines">
                  <For each={denominations()}>
                    {(d) => (
                      <div>
                        <label for={`transfer-line-${d.id}`}>
                          {d.currency} {d.faceValue}
                        </label>
                        <input
                          id={`transfer-line-${d.id}`}
                          type="text"
                          inputMode="numeric"
                          data-denomination={String(d.id)}
                          value={lineQty()[d.id] ?? ''}
                          onInput={(e) => {
                            const value = e.currentTarget.value;
                            setLineQty((prev) => ({ ...prev, [d.id]: value }));
                          }}
                        />
                      </div>
                    )}
                  </For>
                </div>

                <div>
                  <label for="transfer-note">{t('noteLabel')}</label>
                  <input
                    id="transfer-note"
                    name="note"
                    type="text"
                    maxLength={200}
                    value={note()}
                    onInput={(e) => setNote(e.currentTarget.value)}
                  />
                </div>

                <div class="cash-actions">
                  <button
                    type="submit"
                    class="primary"
                    data-action="create"
                    disabled={busy() || kind() === '' || Boolean(meError())}
                    title={meError() ?? undefined}
                  >
                    {t('btnCreateTransfer')}
                  </button>
                </div>
              </form>
            </CardBody>
          </Card>
        </div>
      </Show>
    </div>
  );
}
