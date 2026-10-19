// §9 Recommendations: the list by severity (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import type { JSX } from 'solid-js';
import { For, Show } from 'solid-js';
import type { RecommendationView } from './pricing-board.types';
import './recommendations.css';

export interface RecommendationsProps {
  items: readonly RecommendationView[];
}

const SEVERITY_ORDER: Record<string, number> = {
  High: 0,
  Warn: 1,
  Medium: 2,
  Low: 3,
};

export function Recommendations(props: RecommendationsProps): JSX.Element {
  const sorted = () =>
    [...(props.items ?? [])].sort(
      (a, b) => (SEVERITY_ORDER[a.severity] ?? 99) - (SEVERITY_ORDER[b.severity] ?? 99),
    );

  return (
    <Show when={props.items && props.items.length > 0}>
      <div class="recommendations">
        <For each={sorted()}>
          {(item) => (
            <div class="recommendation-item">
              <span class="recommendation-severity">{item.severity}</span>
              <span class="recommendation-code">{item.code}</span>
              <span class="recommendation-currency">{item.currency ?? ''}</span>
              <span class="recommendation-side">{item.side ?? ''}</span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
