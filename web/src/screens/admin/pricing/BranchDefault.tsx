// V.9 branch default basis points (pricing-tiers.md §15.4): the `"*"` row of pricingBranchPolicySave (PV-d2), used by
// every currency of this branch that has no row of its own.
// Skeleton (FE V.9 skeleton): props and mount point are fixed here; the body is the BranchPolicy ticket's.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingBpDto } from '../../../shared/api/generated/pricing_board';
import type { PricingBranchPolicySaveOutput } from '../../../shared/api/generated/pricing_branch_policy_save';

export interface BranchDefaultProps {
  api: ApiClient;
  branch: string;
  /** `pricing_board.branchDefault`. */
  value: PricingBpDto;
  /** EXECUTIVE at this branch; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingBranchPolicySaveOutput): void;
}

export function BranchDefault(_props: BranchDefaultProps): JSX.Element {
  return null;
}
