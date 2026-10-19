// §9 BulkApprove: count, confirm, calls pricingApprove with the selected rows (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingApproveOutput, PricingRowView } from './pricing-board.types';
import './bulk-approve.css';

export interface BulkApproveProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  /** The rows the user selected; rows with a blocking flag are never sent. */
  rows: readonly PricingRowView[];
  onApproved(out: PricingApproveOutput): void;
}

export function BulkApprove(_props: BulkApproveProps): JSX.Element {
  return null;
}
