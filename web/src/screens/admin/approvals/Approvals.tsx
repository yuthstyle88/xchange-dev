import { createEffect, createSignal, For, onCleanup, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  ApprovalRequestItemDto,
  ApprovalRequestsData,
} from '../../../shared/api/generated/approval_requests';
import { locale, screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Card, CardActions } from '../../../shared/ui/Card';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import {
  decideRefusal,
  decideWrite,
  type DecidePayload,
  type KeyedWrite,
} from '../parts/decide';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './approvals.css';

const t = screenT('admin.approvals', table);

type ApprovalStatusFilter = 'PENDING' | 'APPROVED' | 'REJECTED';
const STATUS_FILTERS: readonly ApprovalStatusFilter[] = ['PENDING', 'APPROVED', 'REJECTED'];

type ApprovalTypeKey = keyof typeof table.th;

const TYPE_LABEL: Record<string, ApprovalTypeKey> = {
  RATE_PUBLICATION: 'ratePublication',
  RATE_FEED_PUBLICATION: 'rateFeedPublication',
  CAMPAIGN_VERSION: 'campaignVersion',
  LOYALTY_RULE_VERSION: 'loyaltyRuleVersion',
  EXCHANGE_TRANSACTION: 'exchangeTransaction',
  TRANSACTION_REVERSAL: 'transactionReversal',
  COUNT_DIFFERENCE: 'countDifference',
  NEGATIVE_STOCK: 'negativeStock',
  CASH_TRANSFER: 'cashTransfer',
};

const APPROVAL_TYPES = [
  'RATE_PUBLICATION',
  'RATE_FEED_PUBLICATION',
  'CAMPAIGN_VERSION',
  'LOYALTY_RULE_VERSION',
  'EXCHANGE_TRANSACTION',
  'TRANSACTION_REVERSAL',
  'COUNT_DIFFERENCE',
  'NEGATIVE_STOCK',
  'CASH_TRANSFER',
] as const;

function statusLabel(status: string): string {
  if (status === 'APPROVED') return t('approved');
  if (status === 'REJECTED') return t('rejected');
  return t('pending');
}

function formatDecidedAt(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

interface OutOfBoundSide {
  currency: string;
  side: 'BUY' | 'SELL';
}

interface RequestCardProps {
  request: ApprovalRequestItemDto;
  branch: string;
  decideWrite: KeyedWrite<DecidePayload, unknown>;
  onDecided: () => Promise<void>;
}

function RequestCard(props: RequestCardProps): JSX.Element {
  let alive = true;
  onCleanup(() => {
    alive = false;
  });

  const [reason, setReason] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const isDecided = (): boolean =>
    props.request.status === 'APPROVED' || props.request.status === 'REJECTED';

  const outOfBound = (): OutOfBoundSide[] => {
    const payload = props.request.proposedPayload;
    if (!payload || typeof payload !== 'object') return [];
    if (Array.isArray(payload.out_of_bound)) return payload.out_of_bound;
    if (Array.isArray(payload.outOfBound)) return payload.outOfBound;
    return [];
  };

  const payloadObject = (): Record<string, unknown> | null => {
    let payload = props.request.proposedPayload;
    if (typeof payload === 'string') {
      try {
        payload = JSON.parse(payload);
      } catch {
        return null;
      }
    }
    if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null;
    return payload as Record<string, unknown>;
  };

  const cashTransferDetail = () => {
    if (props.request.type !== 'CASH_TRANSFER') return null;
    const payload = payloadObject();
    if (!payload) return null;

    const rawTransferId = payload.transfer_id ?? payload.transferId;
    const transferId =
      typeof rawTransferId === 'string' && rawTransferId.trim().length > 0
        ? rawTransferId.trim()
        : null;
    const rawBranch = payload.from_branch ?? payload.fromBranch;
    const fromBranch =
      typeof rawBranch === 'string' && rawBranch.trim().length > 0
        ? rawBranch.trim()
        : null;
    const withinLimit =
      typeof payload.within_limit === 'boolean'
        ? payload.within_limit
        : typeof payload.withinLimit === 'boolean'
          ? payload.withinLimit
          : null;

    if (!transferId && !fromBranch && withinLimit === null) return null;
    return { transferId, fromBranch, withinLimit };
  };

  const reversalDetail = () => {
    if (props.request.type !== 'TRANSACTION_REVERSAL') return null;
    const subj = props.request.subject;
    if (subj?.kind === 'reversal') {
      const customer = subj.customerName
        ? subj.customerNo
          ? `${subj.customerName} (${subj.customerNo})`
          : subj.customerName
        : null;
      return {
        receipt: subj.receiptNumber,
        original: subj.originalTransactionId,
        amount: subj.amountThb,
        why: subj.reason,
        customer,
      };
    }

    const payload = payloadObject();
    if (!payload) return null;

    const rawOrig = payload.originalTransactionId;
    const original =
      typeof rawOrig === 'string' && rawOrig.length > 0 ? rawOrig : null;

    const rawAmount = payload.amountThb;
    const amount =
      typeof rawAmount === 'string' && /^-?\d+(\.\d+)?$/.test(rawAmount)
        ? rawAmount
        : null;

    const rawReason = payload.reason;
    const why =
      typeof rawReason === 'string' && rawReason.length > 0 ? rawReason : null;

    if (!original && !amount && !why) return null;
    return { receipt: null as string | null, original, amount, why, customer: null as string | null };
  };

  const countDetail = () => {
    if (props.request.type !== 'COUNT_DIFFERENCE') return null;
    const subj = props.request.subject;
    if (subj?.kind === 'countDifference') {
      return {
        countedBy: subj.countedBy,
        countedByName: subj.countedByName ?? '—',
        value: subj.netDifferenceThb,
      };
    }

    const payload = payloadObject();
    if (!payload) return null;

    const rawDiff = payload.net_difference_thb;
    const value =
      typeof rawDiff === 'string' && /^-?\d+(\.\d+)?$/.test(rawDiff)
        ? rawDiff
        : null;

    if (value === null) return null;
    return { countedBy: null as string | null, countedByName: null as string | null, value };
  };

  const handleApprove = async () => {
    setError(null);
    const r = reason();
    if (decideRefusal('APPROVED', r) === 'reasonTooLong') {
      const msg = t('reasonTooLong');
      setError(msg);
      toast(msg);
      return;
    }
    setBusy(true);
    try {
      await props.decideWrite.send({
        outcome: 'APPROVED',
        reason: r,
        branch: props.branch,
      });
      if (!alive) return;
      await props.onDecided();
    } catch (err: unknown) {
      if (!alive) return;
      const message = fieldError(err, { reason: t('decisionReason') }).message;
      setError(message);
      toast(message);
    } finally {
      if (alive) {
        setBusy(false);
      }
    }
  };

  const handleReject = async () => {
    setError(null);
    const r = reason();
    const refusal = decideRefusal('REJECTED', r);
    if (refusal === 'reasonRequired') {
      const reqMsg = t('reasonRequired');
      setError(reqMsg);
      toast(reqMsg);
      return;
    }
    if (refusal === 'reasonTooLong') {
      const msg = t('reasonTooLong');
      setError(msg);
      toast(msg);
      return;
    }

    setBusy(true);
    try {
      await props.decideWrite.send({
        outcome: 'REJECTED',
        reason: r,
        branch: props.branch,
      });
      if (!alive) return;
      await props.onDecided();
    } catch (err: unknown) {
      if (!alive) return;
      const message = fieldError(err, { reason: t('decisionReason') }).message;
      setError(message);
      toast(message);
    } finally {
      if (alive) {
        setBusy(false);
      }
    }
  };

  return (
    <div data-request={props.request.id} style="display: contents">
      <Card class="approval-card">
        <div class="approval-card-header">
          <span class="approval-type" data-request-type={props.request.type}>
            {TYPE_LABEL[props.request.type] ? t(TYPE_LABEL[props.request.type]) : props.request.type}
          </span>
          <span class="pill neutral">{statusLabel(props.request.status)}</span>
        </div>

        <div class="approval-proposer">
          <span>{t('requestedBy')}</span>
          <strong title={props.request.requestedBy}>{props.request.requestedByName ?? '—'}</strong>
        </div>

        <Show when={cashTransferDetail()}>
          {(d) => (
            <div class="approval-proposer">
              {d().transferId && <span title={d().transferId!}>#{d().transferId!.slice(0, 8)}</span>}
              {d().transferId && (d().fromBranch || d().withinLimit !== null) && ' · '}
              {d().fromBranch && <span>{d().fromBranch}</span>}
              {d().fromBranch && d().withinLimit !== null && ' · '}
              {d().withinLimit !== null && (
                <span>{d().withinLimit ? t('withinLimit') : t('aboveLimit')}</span>
              )}
            </div>
          )}
        </Show>

        <Show when={reversalDetail()}>
          {(d) => (
            <>
              {d().receipt ? (
                <div class="approval-proposer">
                  <span>{t('receiptNumber')}</span>
                  <span title={d().original ?? undefined}>{d().receipt}</span>
                </div>
              ) : (
                d().original && (
                  <div class="approval-proposer">
                    <span>{t('originalTransaction')}</span>
                    <span title={d().original!}>#{d().original!.slice(0, 8)}</span>
                  </div>
                )
              )}
              {d().amount && (
                <div class="approval-proposer">
                  <span>{t('reversalAmount')}</span>
                  <span>{formatMoney(d().amount!)} THB</span>
                </div>
              )}
              {d().why && (
                <div class="approval-proposer">
                  <span>{t('requestReason')}</span>
                  <span>{d().why}</span>
                </div>
              )}
              {d().customer && (
                <div class="approval-proposer">
                  <span>{t('customer')}</span>
                  <span>{d().customer}</span>
                </div>
              )}
            </>
          )}
        </Show>

        <Show when={countDetail()}>
          {(d) => (
            <>
              {d().countedBy !== null && (
                <div class="approval-proposer">
                  <span>{t('countedBy')}</span>
                  <span title={d().countedBy!}>{d().countedByName}</span>
                </div>
              )}
              <div class="approval-proposer">
                <span>{t('netDifference')}</span>
                <span>{formatMoney(d().value)} THB</span>
              </div>
            </>
          )}
        </Show>

        <Show when={outOfBound().length > 0}>
          <div class="approval-out-of-bound-list">
            <For each={outOfBound()}>
              {(item) => (
                <span class={`pill ${item.side === 'BUY' ? 'buy' : 'sell'}`}>
                  {item.currency}{' '}
                  {item.side === 'BUY' ? t('sideBUY') : t('sideSELL')}
                </span>
              )}
            </For>
          </div>
        </Show>

        <Show
          when={!isDecided()}
          fallback={
            <div data-decision={props.request.id}>
              <Show
                when={props.request.decision}
                fallback={
                  <div class="approval-proposer">{t('noDecisionRecord')}</div>
                }
              >
                {(dec) => (
                  <>
                    <div class="approval-proposer">
                      <span>{t('decidedBy')}</span>
                      <strong title={dec().approverId}>—</strong>
                    </div>
                    <div class="approval-proposer">
                      <span>{t('decisionOutcome')}</span>
                      <strong>{statusLabel(dec().outcome)}</strong>
                    </div>
                    <div class="approval-proposer">
                      <span>{t('decisionReason')}: </span>
                      <span>{dec().reason}</span>
                    </div>
                    <div class="approval-proposer">
                      <span>{t('decidedAt')}</span>
                      <time dateTime={dec().at}>{formatDecidedAt(dec().at)}</time>
                    </div>
                  </>
                )}
              </Show>
            </div>
          }
        >
          <div class="approval-decision-form">
            <label>
              {t('decisionReason')}
              <input
                type="text"
                value={reason()}
                maxLength={500}
                onInput={(e) => setReason(e.currentTarget.value)}
                disabled={busy()}
              />
            </label>
            <div>{reason().length} / 500</div>

            <Show when={error()}>
              <div class="error" role="alert">{error()}</div>
            </Show>

            <CardActions>
              <button
                type="button"
                class="danger"
                onClick={handleReject}
                disabled={busy()}
              >
                {t('reject')}
              </button>
              <button
                type="button"
                class="primary"
                onClick={handleApprove}
                disabled={busy()}
              >
                {t('approve')}
              </button>
            </CardActions>
          </div>
        </Show>
      </Card>
    </div>
  );
}

export function Approvals(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();
  const decideWrites = new Map<
    string,
    KeyedWrite<DecidePayload, unknown>
  >();
  const getDecideWrite = (id: string) => {
    let w = decideWrites.get(id);
    if (!w) {
      w = decideWrite(client, id);
      decideWrites.set(id, w);
    }
    return w;
  };
  const [statusFilter, setStatusFilter] = createSignal<ApprovalStatusFilter>('PENDING');
  const [typeFilter, setTypeFilter] = createSignal<string>('');
  const [requests, setRequests] = createSignal<ApprovalRequestItemDto[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  let reading = 0;

  const loadQueue = async (
    branch: string,
    status: ApprovalStatusFilter = statusFilter(),
    type: string = typeFilter(),
  ) => {
    const mine = ++reading;
    try {
      const filter: { status: string; type?: string } = { status };
      if (type) {
        filter.type = type;
      }
      const res = await client.view<ApprovalRequestsData>('approval_requests', {
        branch,
        filter,
      });
      if (mine !== reading) return;
      setError(null);
      if (res.data?.requests) {
        setRequests(res.data.requests);
      } else {
        setRequests([]);
      }
    } catch (err: unknown) {
      if (mine !== reading) return;
      setError(wordedError(err));
      setRequests([]);
    }
  };

  let lastContext = '';

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    const status = statusFilter();
    const type = typeFilter();
    const context = `${branch}\0${status}\0${type}`;
    if (context !== lastContext) {
      lastContext = context;
      setRequests([]);
      setError(null);
    }
    void loadQueue(branch, status, type);
  });

  return (
    <div class="admin-approvals-screen">
      <div class="approval-out-of-bound-list" data-status-switch>
        <For each={STATUS_FILTERS}>
          {(st) => (
            <button
              type="button"
              class={`pill${statusFilter() === st ? ' active' : ' neutral'}`}
              aria-pressed={statusFilter() === st}
              data-status-filter={st}
              onClick={() => setStatusFilter(st)}
            >
              {statusLabel(st)}
            </button>
          )}
        </For>
      </div>
      <div class="approval-out-of-bound-list" data-type-switch>
        <button
          type="button"
          class={`pill${typeFilter() === '' ? ' active' : ' neutral'}`}
          aria-pressed={typeFilter() === ''}
          data-type-filter=""
          onClick={() => setTypeFilter('')}
        >
          {t('all')}
        </button>
        <For each={APPROVAL_TYPES}>
          {(tp) => (
            <button
              type="button"
              class={`pill${typeFilter() === tp ? ' active' : ' neutral'}`}
              aria-pressed={typeFilter() === tp}
              data-type-filter={tp}
              onClick={() => setTypeFilter(tp)}
            >
              {TYPE_LABEL[tp] ? t(TYPE_LABEL[tp]) : tp}
            </button>
          )}
        </For>
      </div>
      <Show when={error()}>
        <div role="alert" class="error">
          {error()}
        </div>
      </Show>
      <Show
        when={requests().length > 0}
        fallback={
          <Show when={!error()}>
            <Card>
              <div class="empty">
                <strong>{t('empty')}</strong>
              </div>
            </Card>
          </Show>
        }
      >
        <div class="approvals-queue">
          <For each={requests()}>
            {(req) => (
              <RequestCard
                request={req}
                branch={props.branch()}
                decideWrite={getDecideWrite(req.id)}
                onDecided={() => loadQueue(props.branch(), statusFilter(), typeFilter())}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
