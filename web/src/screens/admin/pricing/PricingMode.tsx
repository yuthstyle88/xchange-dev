// V.9 pricing mode panel (pricing-tiers.md §15.1, §15.8a): one currency's mode (AUTO · MANUAL · FOLLOW), its latest
// approved tier set, MAIN's FOLLOW plan result, and at MAIN the FOLLOW ⇄ MANUAL switch (pricingModeSave, PV-f4).
// Skeleton (FE V.9 skeleton): props and mount point are fixed here; the body is the PricingMode ticket's.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type { PricingModeSaveOutput } from '../../../shared/api/generated/pricing_mode_save';

export interface PricingModeProps {
  api: ApiClient;
  branch: string;
  row: PricingRowDto;
  /** `pricing_board.isMain`: the switch exists only at MAIN (the server answers 400 elsewhere). */
  isMain: boolean;
  /** EXECUTIVE at this branch; the switch is hidden otherwise. */
  mayEdit: boolean;
  disabledReason?: string;
  onSaved(out: PricingModeSaveOutput): void;
}

export function PricingMode(_props: PricingModeProps): JSX.Element {
  return null;
}
