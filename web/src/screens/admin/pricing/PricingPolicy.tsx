// §9 PricingPolicy: one currency's override margins and min spread (pricingConfigure) (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingConfigureOutput, PricingRowView } from './pricing-board.types';
import './pricing-policy.css';

export interface PricingPolicyProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowView;
  /** EXECUTIVE frame (pricing-tiers.md §3: override margins are frame); the editor is hidden otherwise. */
  mayEdit: boolean;
  onSaved(out: PricingConfigureOutput): void;
}

export function PricingPolicy(_props: PricingPolicyProps): JSX.Element {
  return null;
}
