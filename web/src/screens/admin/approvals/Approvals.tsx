import { createEffect, createSignal, For, onCleanup, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  ApprovalRequestItemDto,
  ApprovalRequestsData,
} from '../../../shared/api/generated/approval_requests';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { Card, CardActions } from '../../../shared/ui/Card';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './approvals.css';

const t = screenT('admin.approvals', table);

type ApprovalStatusFilter = 'PENDING' | 'APPROVED' | 'REJECTED';
const STATUS_FILTERS: readonly ApprovalStatusFilter[] = ['PENDING', 'APPROVED', 'REJECTED'];

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

interface DecidePayload {
  outcome: 'APPROVED' | 'REJECTED';
  reason: string;
  branch: string;
}

interface RequestCardProps {
  request: ApprovalRequestItemDto;
  branch: string;
  decideWrite: { send(body: DecidePayload): Promise<unknown>; reset(): void };
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

  const handleApprove = async () => {
    setError(null);
    setBusy(true);
    try {
      await props.decideWrite.send({
        outcome: 'APPROVED',
        reason: reason(),
        branch: props.branch,
      });
      if (!alive) return;
      await props.onDecided();
    } catch (err: unknown) {
      if (!alive) return;
      const message = wordedError(err);
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
    if (!r.trim()) {
      const reqMsg = t('reasonRequired');
      setError(reqMsg);
      toast(reqMsg);
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
      const message = wordedError(err);
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
          <span class="approval-type">{t('ratePublication')}</span>
          <span class="pill neutral">{statusLabel(props.request.status)}</span>
        </div>

        <div class="approval-proposer">
          <span>{t('requestedBy')}</span>
          <strong>{props.request.requestedBy}</strong>
        </div>

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
                      <strong>{dec().approverId}</strong>
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
                onInput={(e) => setReason(e.currentTarget.value)}
                disabled={busy()}
              />
            </label>

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
    { send(body: DecidePayload): Promise<unknown>; reset(): void }
  >();
  const getDecideWrite = (id: string) => {
    let w = decideWrites.get(id);
    if (!w) {
      w = keyedWrite<DecidePayload, unknown>(
        client,
        'POST',
        `/api/approvals/${id}/decide`,
      );
      decideWrites.set(id, w);
    }
    return w;
  };
  const [statusFilter, setStatusFilter] = createSignal<ApprovalStatusFilter>('PENDING');
  const [requests, setRequests] = createSignal<ApprovalRequestItemDto[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  let reading = 0;

  const loadQueue = async (branch: string, status: ApprovalStatusFilter = statusFilter()) => {
    const mine = ++reading;
    try {
      const res = await client.view<ApprovalRequestsData>('approval_requests', {
        branch,
        filter: { status },
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
    const context = `${branch}\0${status}`;
    if (context !== lastContext) {
      lastContext = context;
      setRequests([]);
      setError(null);
    }
    void loadQueue(branch, status);
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
                onDecided={() => loadQueue(props.branch(), statusFilter())}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
