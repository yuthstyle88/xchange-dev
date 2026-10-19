// §9 TierEditor: 4 tiers buy/sell prefilled from the suggestion, the margin each makes, approve one currency (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingApproveOutput, PricingRowView, TierBandView } from './pricing-board.types';
import './tier-editor.css';

export interface TierEditorProps {
  api: ApiClient;
  branch: string;
  policyVersionId: string;
  row: PricingRowView;
  bands: readonly TierBandView[];
  onApproved(out: PricingApproveOutput): void;
}

export function TierEditor(_props: TierEditorProps): JSX.Element {
  return null;
}
