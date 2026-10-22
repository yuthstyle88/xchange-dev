// QW-2b (kiosk-shared-phone §5r F7, F8; rows U1, U2, U4, U9 and the expiry of §5r.3; §5r.4a): the /app/queue
// screen's state. Skeleton by QW-2.0; the body is QW-2b's. Create it inside a component (or a reactive root):
// it uses signals.
//
// - `load()`: reads `?branch=<code>` from `search`, then the host's org, the branch's `queue-rates` (branch code →
//   `branchId`) and `my_profile`. `my_profile` 401 ⇒ resolves `{signedOut: true}` (U1) and nothing can be sent.
//   No `?branch=` or a failed read ⇒ phase `failed` with a worded message.
//   `?request=<id>` in `search` (a reload) ⇒ `reissueQr` for a fresh token and phase `open`; a refused reissue
//   (`404`: cancelled, finished or purged) ⇒ phase `form`.
//   Otherwise phase `form`.
// - `submit(v)`: one `create` with `{branch: branchId, ...v}` at a time (a second call while one is in flight sends
//   nothing). `opened` ⇒ phase `open`. `replayed` or `alreadyQueued` ⇒ `reissueQr` for that id, then `open` (U9).
//   A refusal ⇒ `error()` worded, phase stays `form`.
// - Whenever a request opens, the page URL gets `request=<id>` (history.replaceState, other parameters kept), so a
//   reload finds it; going back to the form removes it.
// - `cancel()`: one `cancel` for the open request ⇒ its `status` becomes `CANCELLED` (U4).
// - `expire()`: RequestStatus saw `404` after `expiresAt` ⇒ phase `form` with `expired: true`.
import { createSignal, type Accessor } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import type {
  EmailCopy,
  OpenedRequest,
  QueueApi,
  QueueRate,
  RequestFormValue,
  WebRequestStatus,
} from './queue-api';

const t = screenT('customer.queue', table);

export type QueuePhase =
  | { kind: 'loading' }
  | { kind: 'failed'; message: string }
  | { kind: 'form'; expired: boolean }
  /** `status` is what the page itself knows (CANCELLED after cancel()); null until then. */
  | { kind: 'open'; request: OpenedRequest; status: WebRequestStatus | null };

export interface LoadResult {
  signedOut: boolean;
}

export interface QueueRequestState {
  /** `?branch=` of the page: the branch code ('' when absent). */
  readonly branchCode: string;
  phase: Accessor<QueuePhase>;
  /** The branch id from `queue-rates`, once loaded. */
  branchId: Accessor<string | null>;
  rates: Accessor<QueueRate[]>;
  emailCopy: Accessor<EmailCopy>;
  /** A command (submit, cancel) is in flight. */
  busy: Accessor<boolean>;
  /** The worded refusal of the last submit or cancel; null after a success. */
  error: Accessor<string | null>;
  load(): Promise<LoadResult>;
  submit(value: RequestFormValue): Promise<void>;
  cancel(): Promise<void>;
  expire(): void;
}

export function createQueueRequestState(api: QueueApi, search: string): QueueRequestState {
  const initialParams = new URLSearchParams(search);
  const branchCode = initialParams.get('branch')?.trim() ?? '';
  const initialRequestId = initialParams.get('request')?.trim() ?? '';

  const [phase, setPhase] = createSignal<QueuePhase>({ kind: 'loading' });
  const [branchId, setBranchId] = createSignal<string | null>(null);
  const [rates, setRates] = createSignal<QueueRate[]>([]);
  const [emailCopy, setEmailCopy] = createSignal<EmailCopy>({ available: false, to: null });
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let signedOut = false;

  function syncUrl(requestId: string | null): void {
    if (typeof window === 'undefined' || !window.history?.replaceState) return;
    const params = new URLSearchParams(window.location?.search || search);
    if (branchCode && !params.has('branch')) {
      params.set('branch', branchCode);
    }
    if (requestId) {
      params.set('request', requestId);
    } else {
      params.delete('request');
    }
    const qs = params.toString();
    const pathname = window.location?.pathname || '/app/queue';
    const hash = window.location?.hash || '';
    try {
      window.history.replaceState(null, '', `${pathname}${qs ? `?${qs}` : ''}${hash}`);
    } catch {
      // ignore if history is restricted in test environment
    }
  }

  function openRequest(request: OpenedRequest): void {
    syncUrl(request.requestId);
    setPhase({ kind: 'open', request, status: null });
  }

  function showForm(expired: boolean): void {
    syncUrl(null);
    setPhase({ kind: 'form', expired });
  }

  async function load(): Promise<LoadResult> {
    if (!branchCode) {
      setPhase({ kind: 'failed', message: t('noBranch') });
      return { signedOut: false };
    }
    try {
      const profile = await api.profile();
      const org = await api.host();
      const queueRates = await api.rates(org, branchCode);
      setBranchId(queueRates.branchId);
      setRates(queueRates.rates);
      setEmailCopy(profile.emailCopy);
      if (initialRequestId) {
        try {
          const reopened = await api.reissueQr(queueRates.branchId, initialRequestId);
          openRequest(reopened);
        } catch {
          showForm(false);
        }
      } else {
        showForm(false);
      }
      return { signedOut: false };
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        signedOut = true;
        return { signedOut: true };
      }
      setPhase({ kind: 'failed', message: t('loadFailed') });
      return { signedOut: false };
    }
  }

  async function submit(value: RequestFormValue): Promise<void> {
    const bId = branchId();
    if (signedOut || busy() || !bId) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.create({ branch: bId, ...value });
      if (res.kind === 'opened') {
        openRequest(res.request);
      } else {
        const fresh = await api.reissueQr(bId, res.requestId);
        openRequest(fresh);
      }
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  }

  async function cancel(): Promise<void> {
    const p = phase();
    const bId = branchId();
    if (signedOut || busy() || !bId || p.kind !== 'open' || p.status === 'CANCELLED') return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.cancel(bId, p.request.requestId);
      setPhase({ kind: 'open', request: p.request, status: res.status });
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setBusy(false);
    }
  }

  function expire(): void {
    setError(null);
    showForm(true);
  }

  return {
    branchCode,
    phase,
    branchId,
    rates,
    emailCopy,
    busy,
    error,
    load,
    submit,
    cancel,
    expire,
  };
}
