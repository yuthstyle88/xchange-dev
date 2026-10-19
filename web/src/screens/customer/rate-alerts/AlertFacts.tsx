// C11: an alert card's facts — expired, rate staleness, relation to the target, last event
// (design rate-alerts.md §4.4 `:172`, R11 `:62`; BE contract #3396).
// SKELETON STUB (feat/fe-customer-2): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { MyRateAlertItem } from '../../../shared/api/generated/my_rate_alerts';

/** The fields #3396 adds to `MyRateAlertItem`. Optional until the generated type has them. */
export interface AlertFactsFields {
  relation?: string;
  expiresAt?: string | null;
  expired?: boolean;
  rateStale?: boolean | null;
  lastEvent?: { observedRate: string; triggerAt: string; episodeNo: number } | null;
}

export interface AlertFactsProps {
  alert: MyRateAlertItem & AlertFactsFields;
}

export function AlertFacts(_props: AlertFactsProps): JSX.Element {
  return null;
}
