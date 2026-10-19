// §9 MarketSources: one chip per source: name, age, stale, error; refresh if allowed (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { MarketSourceCode, PricingSource } from './pricing-board.types';
import './market-sources.css';

export interface MarketSourcesProps {
  sources: readonly PricingSource[];
  /** The caller holds `MarketRatesRefresh` (§8). */
  canRefresh: boolean;
  onRefresh(source: MarketSourceCode): void;
}

export function MarketSources(_props: MarketSourcesProps): JSX.Element {
  return null;
}
