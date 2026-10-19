import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatRate } from '../../../shared/ui/rate';
import * as screenI18n from './i18n';
import { lineChange, type RatePublicationLine, type RatePublicationRow, type RateSides } from './rate-history';

const PAGE_SIZE = 20;

export interface RatesHistoryProps {
  client: Pick<ApiClient, 'page'>;
  branch: string;
  /** The first page, already read by the caller (Rates.tsx keeps its toast-on-failure path); when given, the
   *  component does not read it again and pages on from its last row. */
  first?: RatePublicationRow[];
}

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleString(locale());
  } catch {
    return iso;
  }
}

export function RatesHistory(props: RatesHistoryProps): JSX.Element {
  const t = screenT('admin.rates', screenI18n);
  const [rows, setRows] = createSignal<RatePublicationRow[]>(props.first ?? []);
  const [hasMore, setHasMore] = createSignal((props.first?.length ?? 0) === PAGE_SIZE);
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
    if (props.first !== undefined) return;
    void props.client
      .page<RatePublicationRow>('rate_publications', { branch: props.branch, limit: PAGE_SIZE })
      .then((list) => {
        setRows(list);
        setHasMore(list.length === PAGE_SIZE);
        setError(null);
      })
      .catch((err: unknown) => {
        setError(wordedError(err));
      });
  });

  const loadMore = () => {
    const current = rows();
    const last = current[current.length - 1];
    if (!last) return;
    void props.client
      .page<RatePublicationRow>('rate_publications', {
        branch: props.branch,
        limit: PAGE_SIZE,
        before: last.id,
      })
      .then((next) => {
        setRows([...current, ...next]);
        setHasMore(next.length === PAGE_SIZE);
        setError(null);
      })
      .catch((err: unknown) => {
        setError(wordedError(err));
      });
  };

  const renderSides = (sides: RateSides, line: RatePublicationLine) => {
    const buy = sides.buy !== null ? formatRate(sides.buy, line.unitSize, line.currency) : t('notSet');
    const sell = sides.sell !== null ? formatRate(sides.sell, line.unitSize, line.currency) : t('notSet');
    return `${buy} / ${sell}`;
  };

  return (
    <div class="rate-history" data-rate-history="">
      <Show when={error()}>
        {(msg) => (
          <p class="form-error" data-history-error="">
            {msg()}
          </p>
        )}
      </Show>
      <For each={rows()}>
        {(row) => (
          <section data-publication={String(row.version)}>
            <div>
              <strong>#{row.version}</strong>{' '}
              <span>{row.publishedBy}</span>{' '}
              <span>{formatTime(row.publishedAt)}</span>{' '}
              <span class="rates-pill">
                {row.state === 'PUBLISHED' ? t('statePublished') : t('stateSuperseded')}
              </span>
            </div>
            <Show when={row.lines.length > 0}>
              <table class="rates-table">
                <thead>
                  <tr>
                    <th>{t('headerCurrency')}</th>
                    <th>{t('headerBefore')}</th>
                    <th>{t('headerAfter')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={row.lines}>
                    {(line) => (
                      <tr
                        data-history-line={`${line.currency}|${line.group}|${line.valueTier}`}
                        data-line-change={lineChange(line)}
                      >
                        <td>
                          {line.currency} {line.group}
                          <Show when={line.valueTier !== 'ALL'}> {line.valueTier}</Show>
                        </td>
                        <td>{line.before === null ? t('lineAdded') : renderSides(line.before, line)}</td>
                        <td>{line.after === null ? t('lineDropped') : renderSides(line.after, line)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </section>
        )}
      </For>
      <Show when={hasMore()}>
        <button type="button" onClick={loadMore}>
          {t('historyMore')}
        </button>
      </Show>
    </div>
  );
}
