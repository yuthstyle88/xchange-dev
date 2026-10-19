// §9 Recommendations: the list by severity (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { RecommendationView } from './pricing-board.types';
import './recommendations.css';

export interface RecommendationsProps {
  items: readonly RecommendationView[];
}

export function Recommendations(_props: RecommendationsProps): JSX.Element {
  return null;
}
