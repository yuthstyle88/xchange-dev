// QW-2a (kiosk-shared-phone §4.3a W1–W5, W11; §5r F7, F8; §5r.4a): the /app/queue page's wire calls, and the
// types the queue page's parts share. Skeleton by QW-2.0; the bodies are QW-2a's.
//
// Wire (slice c `origin/feat/kwq-c`, slice d `origin/feat/kwq-d`):
// - host():               GET  /api/public/host → {org}
// - rates(org, code):     GET  /api/public/orgs/{org}/branches/{code}/queue-rates → QueueRates (W2)
// - profile():            GET  /api/v3/views/my_profile → data; a signed-out caller is ApiError status 401 (W11)
// - create(input):        POST /api/app/web-requests {branch, type, currency, amount, paymentMethod?}, keyed
//                         → {requestId, requestQrUrl, expiresAt} (W1, W3). `branch` is the branch **id**.
//                         `409 alreadyQueued` names the open request's id in `error.message`.
//                         A replayed create answers `requestQrUrl: null`: the caller reissues.
// - reissueQr(b, id):     POST /api/app/web-requests/{id}/qr {branch}, keyed (a fresh key per call) → as create
// - cancel(b, id):        POST /api/app/web-requests/{id}/cancel {branch}, keyed → {requestId, status: "CANCELLED"}
// - emailCopy(id, token): POST /api/app/copy-by-email {subject: "webRequest", id, qr: token} → {sent, again?}
// Every write goes through ApiClient (keyedWrite / post); never a raw fetch with a write method
// (src/shared/api/raw-write.gate.test.ts) and never a hand-made key (keyed-write.gate.test.ts).
// The token is read from `requestQrUrl` (`/q#<token>`, relative) with `parseRequestQr` (shared/queue/request-qr.ts).
import { ApiClient, ApiError, type Fetch } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { readHostOrg } from '../../../shared/api/public-host';
import { parseRequestQr } from '../../../shared/queue/request-qr';

export type TradeType = 'BUY' | 'SELL';

/** W5. There is no EXPIRED status: an expired request answers 404 (see RequestStatus). */
export type WebRequestStatus =
  | 'QUOTED'
  | 'WAITING'
  | 'CALLED'
  | 'SERVING'
  | 'COMPLETED'
  | 'CANCELLED'
  | 'NO_SHOW';

/** One currency of the kiosk `rates` JSON (W2). */
export interface QueueRate {
  code: string;
  name: string;
  nameEn: string;
  buy: number;
  sell: number;
}

/** The `queue-rates` answer (W2): today's kiosk `rates` JSON. `branchId` is what every command and view takes. */
export interface QueueRates {
  branchId: string;
  branchName: string;
  open: boolean;
  rates: QueueRate[];
  paymentMethods: unknown[];
}

/** W11: `my_profile.emailCopy`. */
export interface EmailCopy {
  available: boolean;
  to: string | null;
}

/** The part of `my_profile` the queue page reads. */
export interface QueueProfile {
  customerId: string;
  displayName: string;
  emailCopy: EmailCopy;
}

/** W1 as slice c amends it: `branch` is the branch id. */
export interface WebRequestInput {
  branch: string;
  type: TradeType;
  currency: string;
  amount: string;
  paymentMethod?: string;
}

/** What the form hands the state; the state adds the branch id. */
export interface RequestFormValue {
  type: TradeType;
  currency: string;
  amount: string;
}

/** A request the page holds, with its QR token in memory only (W3). */
export interface OpenedRequest {
  requestId: string;
  token: string;
  expiresAt: string;
}

export type CreateResult =
  /** A fresh create: the token is in the answer. */
  | { kind: 'opened'; request: OpenedRequest }
  /** A replayed create (`requestQrUrl: null`): the caller calls reissueQr for a token. */
  | { kind: 'replayed'; requestId: string; expiresAt: string }
  /** `409 alreadyQueued`: the member's open request at the branch, by id. */
  | { kind: 'alreadyQueued'; requestId: string };

export interface WebRequestCancelled {
  requestId: string;
  status: 'CANCELLED';
}

/** W4: `my_web_request`'s `data`. */
export interface MyWebRequestData {
  requestId: string;
  status: WebRequestStatus;
  expiresAt: string | null;
  branch: { code: string; name: string };
  type: TradeType;
  currency: string;
  amount: string;
  displayNumber: string | null;
  ticketsAhead: number | null;
}

export interface EmailCopyAnswer {
  sent: boolean;
  again?: boolean;
}

interface RawWebRequestAnswer {
  requestId: string;
  requestQrUrl: string | null;
  expiresAt: string;
}

function readQrToken(requestQrUrl: string | null): string {
  if (!requestQrUrl) {
    throw new Error('Missing requestQrUrl');
  }
  const normalized = requestQrUrl.startsWith('/') ? `http://localhost${requestQrUrl}` : requestQrUrl;
  const token = parseRequestQr(normalized);
  if (!token) {
    throw new Error(`Invalid requestQrUrl: ${requestQrUrl}`);
  }
  return token;
}

export class QueueApi {
  /** The client every call goes through; RequestStatus's ViewPoller reads `my_web_request` through it too. */
  readonly client: ApiClient;
  protected readonly fetchFn: Fetch;
  private readonly writers = new Map<string, { send(body: unknown): Promise<unknown> }>();

  constructor(fetchFn: Fetch) {
    this.fetchFn = fetchFn;
    this.client = new ApiClient(fetchFn);
  }

  private keyed<B, R>(path: string): { send(body: B): Promise<R> } {
    let w = this.writers.get(path);
    if (!w) {
      w = keyedWrite<unknown, unknown>(this.client, 'POST', path);
      this.writers.set(path, w);
    }
    return w as { send(body: B): Promise<R> };
  }

  /** The org of the request's host (`/api/public/host`). Rejects when there is none. */
  host(): Promise<string> {
    return readHostOrg(this.client);
  }

  /** The branch's kiosk rates by branch code (W2). Rejects with ApiError on a non-2xx answer. */
  rates(org: string, code: string): Promise<QueueRates> {
    return this.client.publicGet<QueueRates>(
      `/api/public/orgs/${encodeURIComponent(org)}/branches/${encodeURIComponent(code)}/queue-rates`,
    );
  }

  /** The signed-in member's profile; ApiError status 401 when signed out. */
  async profile(): Promise<QueueProfile> {
    const ans = await this.client.view<QueueProfile>('my_profile');
    if (!ans.data) {
      throw new ApiError({ code: 'notFound', message: 'profile missing', status: 404 });
    }
    return ans.data;
  }

  /** webRequestCreate, keyed. Any refusal other than `409 alreadyQueued` with an id rejects with the ApiError. */
  async create(input: WebRequestInput): Promise<CreateResult> {
    try {
      const ans = await this.keyed<WebRequestInput, RawWebRequestAnswer>('/api/app/web-requests').send(input);
      if (ans.requestQrUrl === null) {
        return { kind: 'replayed', requestId: ans.requestId, expiresAt: ans.expiresAt };
      }
      return {
        kind: 'opened',
        request: {
          requestId: ans.requestId,
          token: readQrToken(ans.requestQrUrl),
          expiresAt: ans.expiresAt,
        },
      };
    } catch (err: unknown) {
      if (err instanceof ApiError && err.status === 409 && err.code === 'alreadyQueued') {
        const id = err.message.trim();
        if (id.length > 0 && id !== 'alreadyQueued' && !/\s/.test(id)) {
          return { kind: 'alreadyQueued', requestId: id };
        }
      }
      throw err;
    }
  }

  /** webRequestQrReissue for `requestId` at branch id `branchId`, keyed with a fresh key per call. */
  async reissueQr(branchId: string, requestId: string): Promise<OpenedRequest> {
    const ans = await this.keyed<{ branch: string }, RawWebRequestAnswer>(
      `/api/app/web-requests/${encodeURIComponent(requestId)}/qr`,
    ).send({ branch: branchId });
    return {
      requestId: ans.requestId,
      token: readQrToken(ans.requestQrUrl),
      expiresAt: ans.expiresAt,
    };
  }

  /** webRequestCancel, keyed. */
  cancel(branchId: string, requestId: string): Promise<WebRequestCancelled> {
    return this.keyed<{ branch: string }, WebRequestCancelled>(
      `/api/app/web-requests/${encodeURIComponent(requestId)}/cancel`,
    ).send({ branch: branchId });
  }

  /** copyByEmail for this web request with the in-memory token. */
  emailCopy(requestId: string, token: string): Promise<EmailCopyAnswer> {
    return this.client.post<{ subject: 'webRequest'; id: string; qr: string }, EmailCopyAnswer>(
      '/api/app/copy-by-email',
      { subject: 'webRequest', id: requestId, qr: token },
    );
  }
}
