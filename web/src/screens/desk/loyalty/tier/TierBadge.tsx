// B03 the customer's tier (FE-desk skeleton stub, #564; view `customer_tier?customer=`,
// api/src/presentation/http/v3/views/customer_tier.rs, type shared/api/generated/customer_tier.ts). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../../shared/api/client';

export interface TierBadgeProps {
  api: ApiClient;
  branch: string;
  customerId: string;
  /** Changes after a phone is verified; the badge reads the view again. */
  refreshKey: number;
}

export function TierBadge(_props: TierBadgeProps): JSX.Element {
  return null;
}
