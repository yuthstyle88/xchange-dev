import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
import type {
  CountSubmitInput,
  HandoverStartInput,
  CashRecordInput,
  CashRecordOutput,
  CountVerifyOutput,
  CashLineInput,
} from '../../../shared/api/generated/cash';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { HandoverPendingRow } from '../../../shared/api/generated/handovers_pending';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { HandoverReceiverRow } from '../../../shared/api/generated/handover_receivers';
import type { StaffCtx } from '../cards';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { th, en } from './i18n';
import './shift.css';

export interface ShiftCardProps {
  ctx: StaffCtx;
}

export function ShiftCard(props: ShiftCardProps): JSX.Element {
  const t = screenT('staff.shift', { th, en });
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
  // #2409: one write at a time; a second click or Enter while one is in flight does nothing.
  const [busy, setBusy] = createSignal<boolean>(false);

  const [cashCounts, setCashCounts] = createSignal<CashCountRow[]>([]);
  const [pendingHandovers, setPendingHandovers] = createSignal<HandoverPendingRow[]>([]);
  const [denominations, setDenominations] = createSignal<DenominationRow[]>([]);
  const [handoverReceivers, setHandoverReceivers] = createSignal<HandoverReceiverRow[]>([]);

  const [countKind, setCountKind] = createSignal<'OPEN' | 'CLOSE' | null>(null);
  const [countQuantities, setCountQuantities] = createSignal<Record<number, string>>({});
  const [showHandover, setShowHandover] = createSignal<boolean>(false);
  const [selectedReceiver, setSelectedReceiver] = createSignal<string>('');
  const [handoverQuantities, setHandoverQuantities] = createSignal<Record<number, string>>({});

  const canOperate = () => can(props.ctx.me, 'ShiftOperate');
  const canCertify = () => can(props.ctx.me, 'ShiftCloseCertify');
  const isClockedIn = () => !!props.ctx.floor?.myClockIn;
  const hasOpenShift = () => !!props.ctx.floor?.myShift;

  const loadCashCounts = async () => {
    if (!canCertify()) return;
    try {
      const res = await api.view<CashCountRow[] | { rows: CashCountRow[] }>('cash_counts');
      const data = res.data;
      if (Array.isArray(data)) {
        setCashCounts(data);
      } else if (data && Array.isArray((data as { rows: CashCountRow[] }).rows)) {
        setCashCounts((data as { rows: CashCountRow[] }).rows);
      }
    } catch (err: unknown) {
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
    } catch (err: unknown) {
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
    } catch (err: unknown) {
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
    } catch (err: unknown) {
      setError(wordedError(err));
    }
  };

  onMount(() => {
    if (canCertify()) {
      void loadCashCounts();
    }
    if (canOperate()) {
      void loadPendingHandovers();
      void loadDenominations();
    }
  });

  const handleOpenCount = (kind: 'OPEN' | 'CLOSE') => {
    setCountKind(kind);
    setShowHandover(false);
    if (denominations().length === 0) {
      void loadDenominations();
    }
  };

  const handleOpenHandover = () => {
    setShowHandover(true);
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

    inputs.forEach((input) => {
      const idStr = input.getAttribute('data-denomination-id');
      if (!idStr) return;
      const d = denominations().find((x) => String(x.id) === idStr);
      const qty = input.value.trim();
      nextQuantities[d ? d.id : +idStr] = input.value;
      if (qty !== '' && d) {
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qty,
        });
      }
    });

    setCountQuantities(nextQuantities);

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
      setError(null);
      setCountKind(null);
      setCountQuantities({});
      await props.ctx.refresh();
      await loadCashCounts();
    } catch (err: unknown) {
      setNotice(null);
      setError(wordedError(err));
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

    inputs.forEach((input) => {
      const idStr = input.getAttribute('data-denomination-id');
      if (!idStr) return;
      const d = denominations().find((x) => String(x.id) === idStr);
      const qty = input.value.trim();
      nextQuantities[d ? d.id : +idStr] = input.value;
      if (qty !== '' && d) {
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qty,
        });
      }
    });

    setHandoverQuantities(nextQuantities);

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
      setError(null);
      setShowHandover(false);
      setSelectedReceiver('');
      setHandoverQuantities({});
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
      setError(wordedError(err));
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
      await cmd.send({ id, branch: props.ctx.me.branchId });
      setNotice(t('saved'));
      setError(null);
      await props.ctx.refresh();
      await loadCashCounts();
    } catch (err: unknown) {
      setNotice(null);
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
      await cmd.send({ id, branch: props.ctx.me.branchId });
      setNotice(t('saved'));
      setError(null);
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
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
      setError(null);
      await props.ctx.refresh();
      await loadPendingHandovers();
    } catch (err: unknown) {
      setNotice(null);
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  };

  const countsToVerify = () => {
    if (!canCertify()) return [];
    return cashCounts().filter(
      (c) =>
        c.verifiedBy === null &&
        (c.kind === 'OPEN' || c.kind === 'CLOSE') &&
        c.countedBy !== props.ctx.me.userId
    );
  };

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
            <button
              type="button"
              data-testid="shift-open-count"
              class="xc-shift-action-btn"
              onClick={() => handleOpenCount('OPEN')}
            >
              {t('openCount')}
            </button>
          </Show>
          <Show when={hasOpenShift()}>
            <button
              type="button"
              data-testid="shift-close-count"
              class="xc-shift-action-btn"
              onClick={() => handleOpenCount('CLOSE')}
            >
              {t('closeCount')}
            </button>
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
          <div class="xc-shift-denoms">
            <For each={denominations()}>
              {(d) => (
                <div data-denomination-row class="xc-shift-denom-row">
                  <span class="xc-shift-denom-label">
                    <span class="xc-shift-denom-currency">{d.currency}</span>{' '}
                    <span class="xc-shift-denom-val">{d.faceValue}</span>
                  </span>
                  <input
                    data-denomination-id={d.id}
                    class="xc-shift-denom-input"
                    type="text"
                    value={countQuantities()[d.id] ?? ''}
                    onInput={(e) => {
                      const v = e.currentTarget.value;
                      setCountQuantities((prev) => ({ ...prev, [d.id]: v }));
                    }}
                    onChange={(e) => {
                      const v = e.currentTarget.value;
                      setCountQuantities((prev) => ({ ...prev, [d.id]: v }));
                    }}
                  />
                </div>
              )}
            </For>
          </div>
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
                <div data-denomination-row class="xc-shift-denom-row">
                  <span class="xc-shift-denom-label">
                    <span class="xc-shift-denom-currency">{d.currency}</span>{' '}
                    <span class="xc-shift-denom-val">{d.faceValue}</span>
                  </span>
                  <input
                    data-denomination-id={d.id}
                    class="xc-shift-denom-input"
                    type="text"
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
                <div data-count-id={c.id} class="xc-shift-count-item">
                  <div class="xc-shift-item-info">
                    <span class="xc-shift-item-title">{c.kind}</span>
                    <span class="xc-shift-item-sub">{c.countedBy}</span>
                  </div>
                  <div class="xc-shift-item-actions">
                    <button
                      type="button"
                      data-testid="shift-verify"
                      class="xc-shift-btn"
                      disabled={busy()}
                      onClick={() => handleVerifyCount(c.id)}
                    >
                      {t('verify')}
                    </button>
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
