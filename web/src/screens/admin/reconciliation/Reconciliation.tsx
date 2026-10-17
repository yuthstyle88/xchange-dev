// FN-1 (#1068): F01, FINANCE's reconcile-and-certify sheet (design `executive-finance.md` §5, §8.2).
// It reads the picked branch's sheet for one business day and certifies exactly the figures on
// screen: certify sends the shown sheet's branch, day and `figuresHash`, so a sheet that moved since
// is refused `staleVersion` by the server.
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { wordedError } from '../../../shared/i18n/worded-error';
import type {
  CertificationRow,
  FinanceReconciliationFilter,
  FinanceSheet,
  Part,
  ReconciliationCertifyInput,
} from '../../../shared/api/generated/finance_reconciliation';
import { locale, screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { refreshTick } from '../parts/refresh';
import * as table from './i18n';
import './reconciliation.css';

const t = screenT('admin.reconciliation', table);

const VIEW = 'finance_reconciliation';
const CERTIFY = '/api/v3/commands/reconciliation_certify';

/** Today's date in Asia/Bangkok, `YYYY-MM-DD`. */
function today(): string {
  const formatter = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Bangkok',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
  return formatter.format(new Date());
}

function stamp(iso: string): string {
  return new Date(iso).toLocaleString(locale(), {
    timeZone: 'Asia/Bangkok',
  });
}

type PartName = 'transactions' | 'counts' | 'transfers' | 'exceptions' | 'integrity' | 'reversalsPending';

/** One part of the sheet: its heading, then its body, or what it waits for when unavailable. */
function PartBox<T>(props: {
  name: PartName;
  part: Part<T>;
  children: (data: T) => JSX.Element;
}): JSX.Element {
  return (
    <section
      class="reconciliation-part"
      data-part={props.name}
      data-unavailable={props.part.available ? undefined : ''}
    >
      <h3>{t(props.name)}</h3>
      <Show
        when={props.part.available && props.part.data !== null}
        fallback={
          <p class="reconciliation-unavailable">
            {t('unavailable')}: {props.part.reason ?? ''}
          </p>
        }
      >
        {props.children(props.part.data as T)}
      </Show>
    </section>
  );
}

function Empty(): JSX.Element {
  return <p class="reconciliation-none">{t('none')}</p>;
}

function Certification(props: { row: CertificationRow }): JSX.Element {
  return (
    <li data-certification={props.row.id} data-stale={props.row.stale ? 'true' : 'false'}>
      <span>{stamp(props.row.certifiedAt)}</span> <span>{props.row.certifiedBy}</span>{' '}
      <span>{props.row.stale ? t('stale') : t('current')}</span>
      <Show when={props.row.note}>
        {' '}
        <span>{props.row.note}</span>
      </Show>
    </li>
  );
}

export function Reconciliation(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const certifyWrite = keyedWrite<ReconciliationCertifyInput, CertificationRow>(api, 'POST', CERTIFY);
  const [sheet, setSheet] = createSignal<FinanceSheet | null>(null);
  const [error, setError] = createSignal<unknown>(null);
  const [note, setNote] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  let reading = 0;

  const day = () => props.period().to || today();

  const read = async (branch: string, date: string) => {
    if (!branch) return;
    const mine = ++reading;
    try {
      const res = await api.view<FinanceSheet, FinanceReconciliationFilter>(VIEW, {
        branch,
        filter: { date },
      });
      if (mine !== reading) return;
      setSheet(res.data ?? null);
    } catch (err: unknown) {
      if (mine !== reading) return;
      setSheet(null);
      setError(err);
    }
  };

  createEffect(() => {
    if (!props.shown()) return;
    refreshTick();
    const branch = props.branch();
    const date = day();
    setError(null);
    setSheet(null);
    void read(branch, date);
  });

  const certify = async () => {
    const shown = sheet();
    if (!shown) return;
    setError(null);
    setBusy(true);
    const input: ReconciliationCertifyInput = {
      branch: shown.branch,
      date: shown.date,
      figuresHash: shown.figuresHash,
      note: note(),
    };
    try {
      await certifyWrite.send(input);
      setNote('');
      if (props.branch() === shown.branch && day() === shown.date) {
        await read(shown.branch, shown.date);
      } else {
        await read(props.branch(), day());
      }
    } catch (err: unknown) {
      if (props.branch() === shown.branch && day() === shown.date) {
        setError(err);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="reconciliation">
      <Show when={error()}>
        <div class="reconciliation-error" role="alert">
          {typeof error() === 'string' ? (error() as string) : wordedError(error())}
        </div>
      </Show>
      <Show when={sheet()}>
        {(s) => (
          <>
            <PartBox name="transactions" part={s().transactions}>
              {(tx) => (
                <>
                  <table class="reconciliation-table">
                    <thead>
                      <tr>
                        <th>{t('method')}</th>
                        <th>{t('buyThb')}</th>
                        <th>{t('sellThb')}</th>
                        <th>{t('fees')}</th>
                        <th>{t('bankFees')}</th>
                        <th>{t('count')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={tx.byMethod}>
                        {(m) => (
                          <tr>
                            <td>{m.method}</td>
                            <td>{m.buyThb}</td>
                            <td>{m.sellThb}</td>
                            <td>{m.fees}</td>
                            <td>{m.bankFees}</td>
                            <td>{m.count}</td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                  <p>
                    {t('reversedThb')}: {tx.reversedThb}
                  </p>
                </>
              )}
            </PartBox>
            <PartBox name="counts" part={s().counts}>
              {(rows) => (
                <Show when={rows.length > 0} fallback={<Empty />}>
                  <ul>
                    <For each={rows}>
                      {(c) => (
                        <li>
                          {c.kind} {c.countedBy} {c.verifiedBy ?? ''}{' '}
                          <For each={c.lines}>
                            {(l) => (
                              <span>
                                {l.currency} {l.counted}/{l.expected} ({l.difference}){' '}
                              </span>
                            )}
                          </For>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              )}
            </PartBox>
            <PartBox name="transfers" part={s().transfers}>
              {(tr) => (
                <ul>
                  <For each={tr.byStatus}>
                    {(b) => (
                      <li>
                        {b.status}: {b.count}
                      </li>
                    )}
                  </For>
                  <For each={tr.inTransit}>
                    {(i) => (
                      <li>
                        {i.transferId} {i.currency} {i.quantity}
                      </li>
                    )}
                  </For>
                  <For each={tr.shortReceipts}>
                    {(r) => (
                      <li>
                        {r.transferId} {r.currency} {r.sent} → {r.received}
                      </li>
                    )}
                  </For>
                </ul>
              )}
            </PartBox>
            <PartBox name="exceptions" part={s().exceptions}>
              {(rows) => (
                <Show when={rows.length > 0} fallback={<Empty />}>
                  <ul>
                    <For each={rows}>{(e) => <li>{e.summary}</li>}</For>
                  </ul>
                </Show>
              )}
            </PartBox>
            <PartBox name="integrity" part={s().integrity}>
              {(rows) => (
                <ul>
                  <For each={rows}>
                    {(r) => (
                      <li>
                        #{r.check} {r.ok ? '✓' : '✗'} {r.offenders}
                      </li>
                    )}
                  </For>
                </ul>
              )}
            </PartBox>
            <PartBox name="reversalsPending" part={s().reversalsPending}>
              {(rows) => (
                <Show when={rows.length > 0} fallback={<Empty />}>
                  <ul>
                    <For each={rows}>
                      {(r) => (
                        <li>
                          {r.requestId} {r.transactionId} {r.requestedBy} {stamp(r.createdAt)}
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              )}
            </PartBox>

            <section class="reconciliation-certify">
              <h3>{t('certifications')}</h3>
              <Show when={s().certifications.length > 0} fallback={<Empty />}>
                <ul>
                  <For each={s().certifications}>{(c) => <Certification row={c} />}</For>
                </ul>
              </Show>
              <label>
                {t('note')}
                <input
                  type="text"
                  value={note()}
                  onInput={(e) => setNote(e.currentTarget.value)}
                />
              </label>
              <button type="button" disabled={busy()} onClick={() => void certify()}>
                {t('certify')}
              </button>
            </section>
          </>
        )}
      </Show>
    </div>
  );
}
