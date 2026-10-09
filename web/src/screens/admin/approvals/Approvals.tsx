import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  ApprovalRequestItemDto,
  ApprovalRequestsData,
} from '../../../shared/api/generated/approval_requests';
import { screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './approvals.css';

const t = screenT('admin.approvals', table);

interface OutOfBoundSide {
  currency: string;
  side: 'BUY' | 'SELL';
}

interface RequestCardProps {
  request: ApprovalRequestItemDto;
  branch: string;
  onDecided: () => Promise<void>;
}

function RequestCard(props: RequestCardProps): JSX.Element {
  const client = new ApiClient();
  const [reason, setReason] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

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
      const key = crypto.randomUUID();
      await client.command(
        'POST',
        `/api/approvals/${props.request.id}/decide`,
        {
          outcome: 'APPROVED',
          reason: reason(),
          branch: props.branch,
        },
        key,
      );
      await props.onDecided();
    } catch (err: unknown) {
      const message = err instanceof ApiError ? err.message : String(err);
      setError(message);
      toast(message);
    } finally {
      setBusy(false);
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
      const key = crypto.randomUUID();
      await client.command(
        'POST',
        `/api/approvals/${props.request.id}/decide`,
        {
          outcome: 'REJECTED',
          reason: r,
          branch: props.branch,
        },
        key,
      );
      await props.onDecided();
    } catch (err: unknown) {
      const message = err instanceof ApiError ? err.message : String(err);
      setError(message);
      toast(message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="approval-card" data-request={props.request.id}>
      <div class="approval-card-header">
        <span class="approval-type">{t('ratePublication')}</span>
        <span class="approval-status">{t('pending')}</span>
      </div>

      <div class="approval-proposer">
        <span>{t('requestedBy')}</span>
        <strong>{props.request.requestedBy}</strong>
      </div>

      <Show when={outOfBound().length > 0}>
        <div class="approval-out-of-bound-list">
          <For each={outOfBound()}>
            {(item) => (
              <div class="approval-out-of-bound-item">
                <span class="currency-code">{item.currency}</span>
                <span class="currency-side">
                  {item.side === 'BUY' ? t('sideBUY') : t('sideSELL')}
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>

      <div class="approval-decision-form">
        <label class="approval-reason-label">
          {t('decisionReason')}
          <input
            type="text"
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
            disabled={busy()}
          />
        </label>

        <Show when={error()}>
          <div class="approval-error-text">{error()}</div>
        </Show>

        <div class="approval-actions">
          <button
            type="button"
            class="approval-btn-approve"
            onClick={handleApprove}
            disabled={busy()}
          >
            {t('approve')}
          </button>
          <button
            type="button"
            class="approval-btn-reject"
            onClick={handleReject}
            disabled={busy()}
          >
            {t('reject')}
          </button>
        </div>
      </div>
    </div>
  );
}

export function Approvals(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();
  const [requests, setRequests] = createSignal<ApprovalRequestItemDto[]>([]);

  const loadQueue = async (branch: string) => {
    try {
      const res = await client.view<ApprovalRequestsData>('approval_requests', {
        branch,
        filter: { status: 'PENDING' },
      });
      if (res.data?.requests) {
        setRequests(res.data.requests);
      } else {
        setRequests([]);
      }
    } catch {
      setRequests([]);
    }
  };

  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    void loadQueue(branch);
  });

  return (
    <div class="admin-approvals-screen">
      <Show
        when={requests().length > 0}
        fallback={<div class="empty-queue-msg">{t('empty')}</div>}
      >
        <div class="approvals-queue">
          <For each={requests()}>
            {(req) => (
              <RequestCard
                request={req}
                branch={props.branch()}
                onDecided={() => loadQueue(props.branch())}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
