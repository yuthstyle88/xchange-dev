// V.9 branch policy for one currency (pricing-tiers.md §15.4, §15.8a): buy/sell basis points, "own" (the branch prices
// this currency itself) and "follow" (enrol in MAIN's FOLLOW), saved with pricingBranchPolicySave (PV-d2).
// Skeleton (FE V.9 skeleton): props and mount point are fixed here; the body is the BranchPolicy ticket's.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type { PricingBranchPolicySaveOutput } from '../../../shared/api/generated/pricing_branch_policy_save';

export interface BranchPolicyProps {
  api: ApiClient;
  branch: string;
  row: PricingRowDto;
  /** `pricing_board.isMain`: MAIN pushes to the branches, so "own" and "follow" mean nothing there. */
  isMain: boolean;
  /** EXECUTIVE at this branch; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingBranchPolicySaveOutput): void;
}

export function BranchPolicy(_props: BranchPolicyProps): JSX.Element {
  return null;
}
