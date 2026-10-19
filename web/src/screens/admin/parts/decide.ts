import type { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';

export type DecideOutcome = 'APPROVED' | 'REJECTED';

export interface DecidePayload {
  outcome: DecideOutcome;
  reason: string;
  branch: string;
}

export type KeyedWrite<B, R> = ReturnType<typeof keyedWrite<B, R>>;

export function decideWrite(
  api: ApiClient,
  requestId: string,
): KeyedWrite<DecidePayload, unknown> {
  return keyedWrite<DecidePayload, unknown>(
    api,
    'POST',
    `/api/approvals/${requestId}/decide`,
  );
}

export function decideRefusal(
  outcome: DecideOutcome,
  reason: string,
): 'reasonRequired' | null {
  if (outcome === 'REJECTED' && reason.trim() === '') {
    return 'reasonRequired';
  }
  return null;
}
