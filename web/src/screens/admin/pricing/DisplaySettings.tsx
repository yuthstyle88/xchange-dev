// V.9 rate-board display settings (pricing-tiers.md §15.6): tiers shown, featured currencies, highlight and
// "all market", saved with rateDisplaySave (PV-j); `{clear: true}` drops this branch's own row.
// Skeleton (FE V.9 skeleton): props and mount point are fixed here; the body is the DisplaySettings ticket's.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingDisplayDto } from '../../../shared/api/generated/pricing_board';
import type { RateDisplaySaveOutput } from '../../../shared/api/generated/rate_display_save';

export interface DisplaySettingsProps {
  api: ApiClient;
  branch: string;
  /** `pricing_board.display`; `from` says whose row it is: `branch` · `MAIN` · `default`. */
  value: PricingDisplayDto;
  /** The board's currency codes, to pick featured ones from. */
  currencies: readonly string[];
  /** EXECUTIVE at this branch; read-only otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: RateDisplaySaveOutput): void;
}

export function DisplaySettings(_props: DisplaySettingsProps): JSX.Element {
  return null;
}
