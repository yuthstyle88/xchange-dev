import { Show, type JSX } from 'solid-js';
import { fill, locale, t } from '../i18n';
import './rate-freshness.css';

export interface RateFreshnessProps {
  stale: boolean;
  sourceDate: string | null | undefined;
  publishedAt: string | null | undefined;
}

export function RateFreshness(props: RateFreshnessProps): JSX.Element {
  const at = () =>
    props.publishedAt
      ? new Date(props.publishedAt).toLocaleString(locale(), {
          timeZone: 'Asia/Bangkok',
        })
      : '';

  return (
    <p class="xc-rate-freshness">
      <Show when={props.stale}>
        <span class="xc-rate-stale" role="status">
          {t('rate.stale')}
        </span>{' '}
      </Show>
      <Show
        when={props.sourceDate}
        fallback={t('rate.noSourceDate')}
      >
        {fill(t('rate.sourceDate'), { date: props.sourceDate! })}
      </Show>
      <Show when={props.publishedAt}>
        {' · '}
        {fill(t('rate.publishedAt'), { at: at() })}
      </Show>
    </p>
  );
}
