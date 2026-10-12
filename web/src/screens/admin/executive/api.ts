// EX-4 (#1186 §7): A01's one read, `GET /api/executive/overview` (EX-3 #1071, design §8.1). `ApiClient` has no
// GET for a non-v3 path, so this calls `fetch` itself, the way the client does, and throws the shared
// `ApiError` so the screen's error path is the other screens'.

import { ApiError } from '../../../shared/api/client';
import type {
  ExecutiveOverviewData,
  ExecutiveOverviewFilter,
  OrgViewAnswer,
} from '../../../shared/api/generated/executive_overview';

export const OVERVIEW_PATH = '/api/executive/overview';

export type ExecutiveOverviewAnswer = OrgViewAnswer<ExecutiveOverviewData>;

/** What the screen asks for. `branches` empty means every branch in scope: no `branches` key is sent. */
export interface OverviewQuery {
  from: string;
  to: string;
  branches: readonly string[];
  channel: NonNullable<ExecutiveOverviewFilter['channel']> | '';
  currency: string;
}

async function refusal(res: Response): Promise<never> {
  const text = await res.text();
  let body: unknown = null;
  try {
    body = JSON.parse(text);
  } catch {
    // not JSON: fall through to the HTTP error
  }
  const error = (body as { error?: unknown } | null)?.error;
  if (error && typeof error === 'object') {
    const e = error as { code?: unknown; field?: unknown; reason?: unknown; message?: unknown };
    throw new ApiError({
      code: typeof e.code === 'string' ? e.code : 'unknown',
      field: typeof e.field === 'string' ? e.field : undefined,
      reason: typeof e.reason === 'string' ? e.reason : undefined,
      message: typeof e.message === 'string' ? e.message : text,
      status: res.status,
    });
  }
  if (typeof error === 'string') {
    throw new ApiError({ code: 'refused', message: error, status: res.status });
  }
  throw new ApiError({ code: 'httpError', message: text || res.statusText, status: res.status });
}

/** One read of the overview. `versions` is sent only on a poll (D-5); a full load sends none. */
export async function fetchOverview(
  q: OverviewQuery,
  versions?: Record<string, number>,
): Promise<ExecutiveOverviewAnswer> {
  const params = new URLSearchParams();
  params.set('from', q.from);
  params.set('to', q.to);
  const branches = q.branches.filter((b) => b !== '');
  if (branches.length > 0) params.set('branches', branches.join(','));
  if (q.channel !== '') params.set('channel', q.channel);
  if (q.currency !== '') params.set('currency', q.currency);
  if (versions !== undefined) params.set('versions', JSON.stringify(versions));
  const res = await fetch(`${OVERVIEW_PATH}?${params.toString()}`, {
    method: 'GET',
    credentials: 'same-origin',
    headers: { 'x-xchange-request': '1' },
  });
  if (!res.ok) await refusal(res);
  return (await res.json()) as ExecutiveOverviewAnswer;
}
