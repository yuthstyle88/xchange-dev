// FN-1 (#1068): F01, FINANCE's reconcile-and-certify sheet (design `executive-finance.md` §5, §8.2).
// It reads the picked branch's sheet for one business day and certifies exactly the figures on
// screen: certify sends the shown sheet's branch, day and `figuresHash`, so a sheet that moved since
// is refused `staleVersion` by the server.
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { fieldError } from '../../../shared/i18n/field-error';
import type {
  CertificationRow,
  FinanceReconciliationFilter,
  FinanceSheet,
  ReconciliationCertifyInput,
} from '../../../shared/api/generated/finance_reconciliation';
import { screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { refreshTick } from '../parts/refresh';
import * as table from './i18n';
import './reconciliation.css';
import { Empty, PartBox, stamp, Who } from './parts';
import { ReversalsPart } from './reversals/ReversalsPart';
import { TransfersPart } from './transfers/TransfersPart';

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





function Certification(props: { row: CertificationRow }): JSX.Element {
  return (
    <li data-certification={props.row.id} data-stale={props.row.stale ? 'true' : 'false'}>
      <span>{stamp(props.row.certifiedAt)}</span> <Who id={props.row.certifiedBy} />{' '}
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
  const singleDay = () => !props.period().from || props.period().from === day();

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
    if (!shown || !singleDay()) return;
    setError(null);
    if (note().length > 500) {
      setError(new ApiError({ status: 400, code: 'validationFailed', field: 'note', message: '' }));
      return;
    }
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

  const resolvedError = () => {
    const err = error();
    if (err === null || err === undefined) return null;
    if (typeof err === 'string') return { message: err };
    return fieldError(err, { note: t('note') });
  };

  return (
    <div class="reconciliation">
      <Show when={resolvedError() && !resolvedError()?.field}>
        <div class="reconciliation-error" role="alert">
          {resolvedError()?.message}
        </div>
      </Show>
      <Show when={sheet()}>
        {(s) => (
          <>
            <header>
              <strong>{t('sheetDay', { date: s().date || day() })}</strong>
              <Show when={!singleDay()}>
                <p class="warning">{t('pickOneDay')}</p>
              </Show>
            </header>
            <PartBox name="transactions" part={s().transactions}>
              {(tx) => (
                <>
                  <Show when={tx.byMethod.length > 0} fallback={<Empty />}>
                    <div class="table-scroll">
                      <table class="reconciliation-table">
                        <thead>
                          <tr>
                            <th>{t('method')}</th>
                            <th class="num">{t('buyThb')}</th>
                            <th class="num">{t('sellThb')}</th>
                            <th class="num">{t('fees')}</th>
                            <th class="num">{t('bankFees')}</th>
                            <th class="num">{t('count')}</th>
                          </tr>
                        </thead>
                        <tbody>
                          <For each={tx.byMethod}>
                            {(m) => (
                              <tr>
                                <td>{m.method}</td>
                                <td class="num">{m.buyThb}</td>
                                <td class="num">{m.sellThb}</td>
                                <td class="num">{m.fees}</td>
                                <td class="num">{m.bankFees}</td>
                                <td class="num">{m.count}</td>
                              </tr>
                            )}
                          </For>
                        </tbody>
                      </table>
                    </div>
                  </Show>
                  <p class="reconciliation-reversed">
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
                          {c.kind} <Who id={c.countedBy} />{' '}
                          <Show when={c.verifiedBy}>{(v) => <Who id={v()} />}</Show>{' '}
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
              {(tr) => <TransfersPart data={tr} />}
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
                <ReversalsPart rows={rows} branch={s().branch} onDecided={() => read(s().branch, s().date)} />
              )}
            </PartBox>

            <section class="card reconciliation-certify">
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
                  name="note"
                  maxLength={500}
                  value={note()}
                  onInput={(e) => setNote(e.currentTarget.value)}
                />
              </label>
              <div class="reconciliation-counter">{note().length} / 500</div>
              <Show when={resolvedError()?.field === 'note'}>
                <div class="reconciliation-error" role="alert" data-field-error="note">
                  {resolvedError()?.message}
                </div>
              </Show>
              <button
                type="button"
                class="primary"
                disabled={busy() || !singleDay()}
                title={!singleDay() ? t('pickOneDay') : undefined}
                onClick={() => void certify()}
              >
                {t('certify')}
              </button>
            </section>
          </>
        )}
      </Show>
    </div>
  );
}
