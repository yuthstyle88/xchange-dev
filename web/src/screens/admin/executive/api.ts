// EX-4 (#1186 §7, #1909): A01's one read, `GET /api/v3/views/executive_overview` (EX-3 #1071, design §8.1),
// read through the shared `ApiClient.view` and its `ApiError` parser.

import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  ExecutiveOverviewData,
  ExecutiveOverviewFilter,
  OrgViewAnswer,
} from '../../../shared/api/generated/executive_overview';

export type ExecutiveOverviewAnswer = OrgViewAnswer<ExecutiveOverviewData>;

/** What the screen asks for. `branches` empty means every branch in scope: no `branches` key is sent. */
export interface OverviewQuery {
  from: string;
  to: string;
  branches: readonly string[];
  channel: NonNullable<ExecutiveOverviewFilter['channel']> | '';
  currency: string;
}

/** One read of the overview. `versions` is sent only on a poll (D-5); a full load sends none. */
export async function fetchOverview(
  q: OverviewQuery,
  versions?: Record<string, number>,
): Promise<ExecutiveOverviewAnswer> {
  const filter: ExecutiveOverviewFilter = {
    from: q.from,
    to: q.to,
    branches: q.branches.filter((b) => b !== '').join(','),
    channel: q.channel || undefined,
    currency: q.currency,
    versions: versions !== undefined ? JSON.stringify(versions) : undefined,
  };
  const answer = await new ApiClient().view<ExecutiveOverviewData, ExecutiveOverviewFilter>(
    'executive_overview',
    { filter },
  );
  if (!answer.versions) {
    throw new ApiError({
      code: 'httpError',
      message: 'missing versions in executive_overview',
      status: 200,
    });
  }
  const out: ExecutiveOverviewAnswer = {
    changed: answer.changed,
    versions: answer.versions,
  };
  if ('data' in answer && answer.data !== undefined) {
    out.data = answer.data;
  }
  return out;
}
