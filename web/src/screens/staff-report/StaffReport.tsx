// V3-staff-report #1350: /staff/report (design `xchange-v3-wave3-staff-report.md` §B.1 rows 1–22, D-B1, D-B4). v2's
// page is the look; every figure is the server's `staff_daily_report` answer, printed as given: the screen sums,
// counts and filters nothing. It reads on mount (no day: the server decides today) and on a day change, never on a
// timer.
import { createSignal, For, Match, onMount, Show, Switch, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../shared/api/client';
import type {
  ReportCurrencyRow,
  ReportStatus,
  StaffDailyReportData,
  StaffDailyReportFilter,
} from '../../shared/api/generated/staff_daily_report';
import { lang, screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { formatMoney } from '../../shared/ui/MoneyText';
import * as table from './i18n';
import './staff-report.css';

const t = screenT('staff-report', table);
const VIEW = 'staff_daily_report';

type State =
  | { kind: 'loading' }
  | { kind: 'refused'; message: string }
  | { kind: 'ready'; report: StaffDailyReportData };

const STATUS: Record<ReportStatus, 'reconciled' | 'closedAwaitingReconciliation' | 'preliminary'> = {
  reconciled: 'reconciled',
  closedAwaitingReconciliation: 'closedAwaitingReconciliation',
  preliminary: 'preliminary',
};

/** A stored figure, or v2's "not counted" word when the server has none. */
function figure(value: string | null): string {
  return value === null ? t('waitingCount') : formatMoney(value);
}

/** `generatedAt` in the branch's timezone the server answered (B.1 row 20), not the browser's. */
function generated(at: string, timeZone: string): string {
  const locale = lang() === 'th' ? 'th-TH' : 'en-GB';
  try {
    return new Date(at).toLocaleString(locale, { timeZone });
  } catch {
    return at;
  }
}

function CurrencyRow(props: { row: ReportCurrencyRow }): JSX.Element {
  return (
    <tr>
      <td>{props.row.currency}</td>
      <td>{figure(props.row.drawerCounted)}</td>
      <td>{figure(props.row.safeCounted)}</td>
      <td>{figure(props.row.drawerDifference)}</td>
      <td>{figure(props.row.safeDifference)}</td>
    </tr>
  );
}

function Sheet(props: { report: StaffDailyReportData }): JSX.Element {
  const r = () => props.report;
  const transit = () =>
    r().inTransit.length === 0
      ? t('none')
      : r()
          .inTransit.map((x) => `${x.kind} ${formatMoney(x.amount)} ${x.currency}`)
          .join(' / ');
  const signers = () =>
    r().signers.length === 0
      ? t('signersFallback')
      : r()
          .signers.map((s) => `${s.name} (${t(s.capacity)})`)
          .join(' / ');
  return (
    <article class="staff-report-sheet">
      <header class="staff-report-heading">
        <p class="staff-report-eyebrow">{t('sheetTitle')}</p>
        <h2>{r().company ?? t('noEntity')}</h2>
        <h3>{t('siteDay', { site: r().site ?? r().branchName, day: r().day })}</h3>
        <p class="staff-report-status">
          {t('statusLine', { status: t(STATUS[r().status]), branch: r().branchName })}
        </p>
      </header>

      <section class="staff-report-metrics">
        <p>
          {t('completed')} <b>{r().completed}</b>
        </p>
        <p>
          {t('buyVolume')} <b>{formatMoney(r().buyVolume)}</b> THB
        </p>
        <p>
          {t('sellVolume')} <b>{formatMoney(r().sellVolume)}</b> THB
        </p>
        <p>
          {t('reversedVolume')} <b>{formatMoney(r().reversedVolume)}</b> THB
        </p>
        <p>
          {t('pendingTransactions')} <b>{r().pendingTransactions}</b>
        </p>
      </section>

      <table class="staff-report-table">
        <thead>
          <tr>
            <th>{t('currency')}</th>
            <th>{t('drawerCounted')}</th>
            <th>{t('safeCounted')}</th>
            <th>{t('drawerDifference')}</th>
            <th>{t('safeDifference')}</th>
          </tr>
        </thead>
        <tbody>
          <For each={r().currencies}>{(row) => <CurrencyRow row={row} />}</For>
        </tbody>
      </table>

      <section class="staff-report-checks">
        <h3>{t('handoverHeading')}</h3>
        <p>{t('requestsLine', { requests: r().pendingRequests, exceptions: r().openExceptions })}</p>
        <For each={r().notes}>
          {(n) => (
            <p class="staff-report-note">
              {n.kind}: {n.note}
            </p>
          )}
        </For>
        <p>{t('holdsLine', { holds: r().openHolds })}</p>
        <p>{t('inTransitLine', { list: transit() })}</p>
      </section>

      <section class="staff-report-signers">
        <h3>{t('signersHeading')}</h3>
        <p>{signers()}</p>
        <p>{t('finance', { note: r().financeNote ?? t('financeFallback') })}</p>
      </section>

      <footer class="staff-report-footer">
        <p>{t('fixedNote')}</p>
        <p>{t('footer', { at: generated(r().generatedAt, r().timezone) })}</p>
      </footer>
    </article>
  );
}

export function StaffReport(): JSX.Element {
  const api = new ApiClient();
  const [state, setState] = createSignal<State>({ kind: 'loading' });
  const [day, setDay] = createSignal('');

  async function read(chosen?: string): Promise<void> {
    setState({ kind: 'loading' });
    const filter: StaffDailyReportFilter = chosen ? { day: chosen } : {};
    try {
      const answer = await api.view<StaffDailyReportData, StaffDailyReportFilter>(VIEW, { filter });
      if (answer.data) {
        setDay(answer.data.day);
        setState({ kind: 'ready', report: answer.data });
      }
    } catch (e) {
      const message =
        e instanceof ApiError && e.code === 'roleNotAllowed'
          ? t('roleNotAllowed')
          : wordedError(e);
      setState({ kind: 'refused', message });
    }
  }

  onMount(() => {
    void read();
  });

  return (
    <div class="staff-report">
      <div class="staff-report-tools">
        <label>
          {t('day')}{' '}
          <input
            type="date"
            value={day()}
            onChange={(e) => {
              const v = e.currentTarget.value;
              setDay(v);
              if (v) void read(v);
            }}
          />
        </label>
        <button type="button" onClick={() => window.print()}>
          {t('print')}
        </button>
        <a href="/staff">{t('back')}</a>
      </div>
      <Switch>
        <Match when={state().kind === 'loading'}>
          <p class="staff-report-loading">{t('loading')}</p>
        </Match>
        <Match when={state().kind === 'refused'}>
          <div class="staff-report-refusal">
            <p>{(state() as { message: string }).message}</p>
            <a href="/staff">{t('signInAgain')}</a>
          </div>
        </Match>
        <Match when={state().kind === 'ready'}>
          <Show when={(state() as { report?: StaffDailyReportData }).report}>
            {(report) => <Sheet report={report()} />}
          </Show>
        </Match>
      </Switch>
    </div>
  );
}
