import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { RatePublicationRow } from './rate-history';

export interface RatesHistoryProps {
  client: Pick<ApiClient, 'page'>;
  branch: string;
  /** The first page, already read by the caller (Rates.tsx keeps its toast-on-failure path); when given, the
   *  component does not read it again and pages on from its last row. */
  first?: RatePublicationRow[];
}

/** Skeleton stub (feat/fe-pricing); the A03 history ticket implements it. */
export function RatesHistory(_props: RatesHistoryProps): JSX.Element {
  return <div class="rate-history" data-rate-history="" />;
}
