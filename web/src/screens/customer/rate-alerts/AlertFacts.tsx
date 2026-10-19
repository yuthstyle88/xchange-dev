// C11: an alert card's facts — expired, rate staleness, relation to the target, last event
// (design rate-alerts.md §4.4 `:172`, R11 `:62`; BE contract #3396).
import { Show, type JSX } from 'solid-js';
import type { MyRateAlertItem } from '../../../shared/api/generated/my_rate_alerts';
import { locale, screenT } from '../../../shared/i18n';
import { MoneyText } from '../../../shared/ui/MoneyText';
import * as table from './i18n';

const t = screenT('customer.rate-alerts', table);

function formatRateTime(isoString: string): string {
  try {
    return new Date(isoString).toLocaleString(locale(), {
      timeZone: 'Asia/Bangkok',
    });
  } catch {
    return isoString;
  }
}

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

export function AlertFacts(props: AlertFactsProps): JSX.Element {
  const isExpired = () => props.alert.expired === true;
  const hasExpires = () => !isExpired() && typeof props.alert.expiresAt === 'string' && props.alert.expiresAt.length > 0;
  const isStale = () => props.alert.rateStale === true;
  const isRelationKnown = () => props.alert.relation === 'SATISFIED' || props.alert.relation === 'NOT_SATISFIED';
  const hasLastEvent = () => props.alert.lastEvent !== null && props.alert.lastEvent !== undefined;

  const hasAnyFact = () => isExpired() || hasExpires() || isStale() || isRelationKnown() || hasLastEvent();

  return (
    <Show when={hasAnyFact()}>
      <div class="xc-alert-facts">
        <Show when={isExpired()}>
          <span class="xc-alert-expired" data-testid="alert-expired">
            {t('expired')}
          </span>
        </Show>
        <Show when={hasExpires()}>
          <span class="xc-alert-expires" data-testid="alert-expires">
            {t('expiresOn', { at: formatRateTime(props.alert.expiresAt!) })}
          </span>
        </Show>
        <Show when={isStale()}>
          <span class="xc-alert-stale" data-testid="alert-stale">
            {t('rateStale')}
          </span>
        </Show>
        <Show when={isRelationKnown()}>
          <span class="xc-alert-relation" data-testid="alert-relation">
            {props.alert.relation === 'SATISFIED' ? t('relationSatisfied') : t('relationNotSatisfied')}
          </span>
        </Show>
        <Show when={props.alert.lastEvent}>
          {(ev) => {
            const text = () => t('lastEvent', { at: formatRateTime(ev().triggerAt), rate: '__RATE__' });
            const parts = () => text().split('__RATE__');
            return (
              <span class="xc-alert-last-event" data-testid="alert-last-event">
                {parts()[0]}
                <MoneyText value={ev().observedRate} currency="THB" />
                {parts()[1]}
              </span>
            );
          }}
        </Show>
      </div>
    </Show>
  );
}
