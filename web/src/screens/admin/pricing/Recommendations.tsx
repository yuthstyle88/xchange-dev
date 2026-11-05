// §9 Recommendations: the list by severity, each code in words (TH/EN) with its raw code in `title` for staff who know
// it (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §5.2, §9).
import type { JSX } from 'solid-js';
import { For, Show } from 'solid-js';
import type { PricingRecommendationDto } from '../../../shared/api/generated/pricing_board';
import { lang } from '../../../shared/i18n';
import { en, th } from './i18n';
import { NOT_OFFERED } from './pricing-format';
import './recommendations.css';

export interface RecommendationsProps {
  items: readonly PricingRecommendationDto[];
}

const SEVERITY_ORDER: Record<string, number> = {
  High: 0,
  Warn: 1,
  Medium: 2,
  Low: 3,
};

/** `prefix.code` in the page language; an unknown code shows as sent. */
function worded(prefix: 'rec' | 'severity' | 'side', code: string): string {
  const tbl = lang() === 'en' ? en : th;
  return tbl[`${prefix}.${code}` as keyof typeof th] ?? code;
}

export function Recommendations(props: RecommendationsProps): JSX.Element {
  const sorted = () =>
    [...(props.items ?? [])]
      .filter((i) => i.code !== NOT_OFFERED)
      .sort((a, b) => (SEVERITY_ORDER[a.severity] ?? 99) - (SEVERITY_ORDER[b.severity] ?? 99));
  /** `NotOffered` comes one row per currency (pricing-tiers.md §5.2); it reads as one line listing them. */
  const notOffered = () => (props.items ?? []).filter((i) => i.code === NOT_OFFERED);
  const title = () => (lang() === 'en' ? en : th).recommendations;

  return (
    <Show when={props.items && props.items.length > 0}>
      <section class="recommendations" aria-label={title()}>
        <h3 class="recommendations-title">
          {title()} <span class="recommendations-count">{props.items.length}</span>
        </h3>
        <ul class="recommendations-list">
          <For each={sorted()}>
            {(item) => (
              <li class="recommendation-item" data-severity={item.severity} data-code={item.code}>
                <span class="recommendation-severity" data-severity={item.severity}>
                  {worded('severity', item.severity)}
                </span>
                <span class="recommendation-currency">{item.currency ?? ''}</span>
                <span class="recommendation-code" title={item.code}>
                  {worded('rec', item.code)}
                </span>
                <span class="recommendation-side" data-side={item.side ?? ''}>
                  {item.side ? worded('side', item.side) : ''}
                </span>
              </li>
            )}
          </For>
          <Show when={notOffered().length > 0}>
            <li class="recommendation-item recommendation-item-group" data-severity={notOffered()[0]!.severity} data-code={NOT_OFFERED}>
              <span class="recommendation-severity" data-severity={notOffered()[0]!.severity}>
                {worded('severity', notOffered()[0]!.severity)}
              </span>
              <span class="recommendation-code" title={NOT_OFFERED}>
                {worded('rec', NOT_OFFERED)} ({notOffered().length})
              </span>
              <span class="recommendation-currency recommendation-currency-list">
                {notOffered()
                  .map((i) => i.currency ?? '')
                  .filter(Boolean)
                  .join(' · ')}
              </span>
            </li>
          </Show>
        </ul>
      </section>
    </Show>
  );
}
