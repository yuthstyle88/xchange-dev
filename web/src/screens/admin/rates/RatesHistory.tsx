import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';

export interface RatesHistoryProps {
  client: Pick<ApiClient, 'page'>;
  branch: string;
}

/** Skeleton stub (feat/fe-pricing); the A03 history ticket implements it. */
export function RatesHistory(_props: RatesHistoryProps): JSX.Element {
  return <div class="rate-history" data-rate-history="" />;
}
