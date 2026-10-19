// §9 PricingGrid: the table, enabled currencies first, others folded below (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { PricingRowView } from './pricing-board.types';
import './pricing-grid.css';

export interface PricingGridProps {
  /** Already filtered by `Pricing`; the grid lists enabled currencies first and folds the others below. */
  rows: readonly PricingRowView[];
  selected: ReadonlySet<string>;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
}

export function PricingGrid(_props: PricingGridProps): JSX.Element {
  return null;
}
