// §9 PricingRow: code, TT / SR / BoT, current shop rate, suggested tier 1, status chip, flags, select box (disabled when a flag blocks) (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { PricingRowView } from './pricing-board.types';
import './pricing-row.css';

export interface PricingRowProps {
  row: PricingRowView;
  selected: boolean;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
}

export function PricingRow(_props: PricingRowProps): JSX.Element {
  return null;
}
