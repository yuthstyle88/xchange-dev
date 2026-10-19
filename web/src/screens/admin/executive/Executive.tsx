// EX-4 (#1186): A01, every branch at once (design `docs/design/blueprint/executive-finance.md` §4.1–§4.5, §8.1).
// The screen computes no figure (D-2, D-3): each number, each "unavailable" and each drill-down is EX-3's
// answer, and money is the server's decimal string through `formatMoney`.
//
// Reads: a full load (no `versions`) on show, on a new period and on every filter change; a poll every 60 s,
// while shown and visible, with the versions it holds (D-5). A poll answering `changed:false` touches nothing;
// one that names moved branches replaces only their rows and alerts, and the KPIs.

import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount, type JSX } from 'solid-js';
import type {
  AlertDto,
  BranchRowDto,
  DefinitionDto,
  DrilldownDto,
  ExecutiveOverviewData,
  MetricDto,
} from '../../../shared/api/generated/executive_overview';
import { lang, locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { fetchOverview, type OverviewQuery } from './api';
import { drilldownHash } from './drilldown';
import * as table from './i18n';
import './executive.css';

const t = screenT('admin.executive', table);

type Key = keyof typeof table.th;
type Reason = NonNullable<MetricDto['unavailable']>;
type Channel = OverviewQuery['channel'];

const POLL_MS = 60_000;

/** E-1's label of each KPI key (design §3). A key EX-3 adds later shows as itself. */
const KPI_LABEL: Record<string, Key> = {
  buy_volume_thb: 'kpiBuy',
  sell_volume_thb: 'kpiSell',
  realized_margin_thb: 'kpiMargin',
  repeat_customers: 'kpiRepeat',
  booking_conversion: 'kpiConversion',
  commission_thb: 'kpiCommission',
};

const REASON: Record<Reason, Key> = {
  awaitsLedger: 'awaitsLedger',
  awaitsBookingDesign: 'awaitsBookingDesign',
  awaitsBroker: 'awaitsBroker',
  awaitsCampaigns: 'awaitsCampaigns',
  noBookingsInPeriod: 'noBookingsInPeriod',
};

/** The four wire kinds of #1071 and EX-13 (#1077); any other kind shows its raw key. */
const ALERT_LABEL: Record<string, Key> = {
  ratePending: 'alertRatePending',
  staleData: 'alertStaleData',
  countDifference: 'alertCountDifference',
  lowStock: 'alertLowStock',
};

const CHANNELS: { value: Channel; label: Key }[] = [
  { value: '', label: 'channelAll' },
  { value: 'WalkIn', label: 'channelWalkIn' },
  { value: 'Booking', label: 'channelBooking' },
  { value: 'Kiosk', label: 'channelKiosk' },
  { value: 'Referred', label: 'channelReferred' },
];

interface Filters {
  branches: string[];
  channel: Channel;
  currency: string;
}

/** A KPI that is money (`*_thb`, E-1); the others (a count, a rate) are shown as sent. */
function isMoney(key: string): boolean {
  return key.endsWith('_thb');
}

function Drilldown(props: { drilldown: DrilldownDto | null }): JSX.Element {
  const hash = () => (props.drilldown ? drilldownHash(props.drilldown) : null);
  return (
    <Show when={hash()}>
      {(h) => (
        <span class="executive-drill">
          <button
            type="button"
            class="executive-drill-link"
            data-drilldown
            onClick={() => {
              location.hash = h();
            }}
          >
            {t('open')}
          </button>
          <Show when={props.drilldown!.dropped.length > 0}>
            <span class="executive-dropped">
              {t('dropped')}: {props.drilldown!.dropped.join(', ')}
            </span>
          </Show>
        </span>
      )}
    </Show>
  );
}

/** A metric's figure, or the fixed sentence of why there is none (D-2): never a number for a missing one. */
function MetricValue(props: { metric: MetricDto; money: boolean; class: string }): JSX.Element {
  return (
    <Show
      when={props.metric.unavailable === null && props.metric.value !== null}
      fallback={
        <span class="executive-unavailable">
          {props.metric.unavailable ? t(REASON[props.metric.unavailable]) : ''}
        </span>
      }
    >
      <span class={props.class}>{props.money ? formatMoney(props.metric.value!) : props.metric.value!}</span>
    </Show>
  );
}

/** A server time (UTC ISO-8601) as Bangkok date and time in the screen's locale; an unparsable one as sent. */
function bangkok(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function Time(props: { at: string | null }): JSX.Element {
  return (
    <Show when={props.at} fallback={<span class="executive-never">{t('never')}</span>}>
      {(at) => <time datetime={at()}>{bangkok(at())}</time>}
    </Show>
  );
}

export function Executive(props: AdminMenuProps): JSX.Element {
  const [kpis, setKpis] = createSignal<MetricDto[]>([]);
  const [rows, setRows] = createSignal<BranchRowDto[]>([]);
  const [alerts, setAlerts] = createSignal<AlertDto[]>([]);
  const [definitions, setDefinitions] = createSignal<DefinitionDto[]>([]);
  const [known, setKnown] = createSignal<{ id: string; name: string }[]>([]);
  const [filters, setFilters] = createSignal<Filters>({ branches: [], channel: '', currency: '' });
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [buyReversed, setBuyReversed] = createSignal<string | null>(null);
  const [sellReversed, setSellReversed] = createSignal<string | null>(null);

  let versions: Record<string, number> | null = null;
  let latest = 0;

  const query = (): OverviewQuery => {
    const p = props.period();
    const f = filters();
    return { from: p.from, to: p.to, branches: f.branches, channel: f.channel, currency: f.currency };
  };

  const remember = (branches: BranchRowDto[]) => {
    const seen = known();
    const ids = new Set(seen.map((b) => b.id));
    const added = branches.filter((b) => !ids.has(b.branchId)).map((b) => ({ id: b.branchId, name: b.name }));
    if (added.length > 0) setKnown([...seen, ...added]);
  };

  const clear = () => {
    setKpis([]);
    setRows([]);
    setAlerts([]);
    setBuyReversed(null);
    setSellReversed(null);
    versions = null;
  };

  /** Everything the answer says, replacing what is on screen. */
  const show = (d: ExecutiveOverviewData) => {
    setKpis(d.kpis);
    setRows(d.branches);
    setAlerts(d.alerts);
    setDefinitions(d.definitions);
    setBuyReversed(d.buyReversedThb);
    setSellReversed(d.sellReversedThb);
    remember(d.branches);
  };

  /** A poll's answer: only the branches it lists moved (D-5). The rest keep their rows and alerts. */
  const merge = (d: ExecutiveOverviewData, inScope: Set<string>) => {
    const moved = new Map(d.branches.map((b) => [b.branchId, b]));
    const kept = rows().filter((r) => inScope.has(r.branchId));
    const next = kept.map((r) => moved.get(r.branchId) ?? r);
    const present = new Set(next.map((r) => r.branchId));
    for (const b of d.branches) if (!present.has(b.branchId)) next.push(b);
    setRows(next);
    setAlerts([
      ...alerts().filter((a) => inScope.has(a.branchId) && !moved.has(a.branchId)),
      ...d.alerts,
    ]);
    setKpis(d.kpis);
    setDefinitions(d.definitions);
    setBuyReversed(d.buyReversedThb);
    setSellReversed(d.sellReversedThb);
    remember(d.branches);
  };

  const load = async () => {
    const mine = ++latest;
    try {
      const answer = await fetchOverview(query());
      if (mine !== latest) return;
      setRefusal(null);
      versions = answer.versions;
      if (answer.data) show(answer.data);
      setLoaded(true);
    } catch (e) {
      if (mine !== latest) return;
      clear();
      setRefusal(wordedError(e));
    }
  };

  const poll = async () => {
    if (!props.shown() || document.hidden) return;
    if (versions === null) {
      await load();
      return;
    }
    const mine = ++latest;
    try {
      const answer = await fetchOverview(query(), versions);
      if (mine !== latest) return;
      versions = answer.versions;
      if (!answer.changed || !answer.data) return;
      merge(answer.data, new Set(Object.keys(answer.versions)));
    } catch (e) {
      if (mine !== latest) return;
      clear();
      setRefusal(wordedError(e));
    }
  };

  createEffect(
    on([props.shown, props.period, filters], ([shown]) => {
      if (shown) {
        versions = null;
        void load();
      }
    }),
  );

  onMount(() => {
    const timer = setInterval(() => void poll(), POLL_MS);
    onCleanup(() => clearInterval(timer));
  });

  const definition = (key: string): string => {
    const d = definitions().find((x) => x.key === key);
    if (!d) return '';
    return lang() === 'en' ? d.en : d.th;
  };

  const branchName = (id: string): string => known().find((b) => b.id === id)?.name ?? id;

  /** §4.2: the oldest `as_of` of the branches shown. ISO-8601 UTC strings order as text. */
  const oldest = createMemo(() => {
    const times = rows()
      .map((r) => r.asOf)
      .filter((a): a is string => a !== null);
    return times.length === 0 ? null : times.reduce((a, b) => (b < a ? b : a));
  });

  const period = () => {
    const p = props.period();
    return p.from === p.to ? p.from : `${p.from} – ${p.to}`;
  };

  const toggleBranch = (id: string, checked: boolean) => {
    const f = filters();
    const chosen = checked ? [...f.branches.filter((b) => b !== id), id] : f.branches.filter((b) => b !== id);
    const order = known().map((b) => b.id);
    chosen.sort((a, b) => order.indexOf(a) - order.indexOf(b));
    setFilters({ ...f, branches: chosen });
  };

  return (
    <div class="executive">
      <header class="executive-header">
        <span class="executive-crumb">XC</span>
        <span class="executive-crumb">{t('allBranches')}</span>
        <span class="executive-crumb">{period()}</span>
        <span class="executive-crumb">
          {t('latest')} <Time at={oldest()} />
        </span>
      </header>

      <div class="executive-filters">
        <fieldset class="executive-branch-filter">
          <legend>{t('filterBranches')}</legend>
          <For each={known()}>
            {(b) => (
              <label class="executive-check">
                <input
                  type="checkbox"
                  name="branch"
                  value={b.id}
                  checked={filters().branches.includes(b.id)}
                  onChange={(e) => toggleBranch(b.id, e.currentTarget.checked)}
                />
                {b.name}
              </label>
            )}
          </For>
        </fieldset>
        <label class="executive-field">
          {t('filterCurrency')}
          <input
            name="currency"
            maxLength={3}
            value={filters().currency}
            onChange={(e) => setFilters({ ...filters(), currency: e.currentTarget.value.trim().toUpperCase() })}
          />
        </label>
        <label class="executive-field">
          {t('filterChannel')}
          <select
            name="channel"
            value={filters().channel}
            onChange={(e) => setFilters({ ...filters(), channel: e.currentTarget.value as Channel })}
          >
            <For each={CHANNELS}>{(c) => <option value={c.value}>{t(c.label)}</option>}</For>
          </select>
        </label>
      </div>

      <Show when={refusal()}>
        {(r) => (
          <div class="error-banner" role="alert">
            {r()}
          </div>
        )}
      </Show>

      <Show when={refusal() === null}>
        <Show when={loaded()} fallback={<p class="executive-loading">{t('loading')}</p>}>
          <section class="executive-kpis">
            <For each={kpis()}>
              {(k) => (
                <article class="executive-kpi" data-kpi={k.key}>
                  <h3 class="executive-kpi-label">{KPI_LABEL[k.key] ? t(KPI_LABEL[k.key]!) : k.key}</h3>
                  <MetricValue metric={k} money={isMoney(k.key)} class="executive-kpi-value" />
                  <Show when={k.key === 'buy_volume_thb' && buyReversed() !== null}>
                    <span class="executive-kpi-reversed">
                      {t('reversed')}: {buyReversed()}
                    </span>
                  </Show>
                  <Show when={k.key === 'sell_volume_thb' && sellReversed() !== null}>
                    <span class="executive-kpi-reversed">
                      {t('reversed')}: {sellReversed()}
                    </span>
                  </Show>
                  <p class="executive-kpi-definition">{definition(k.key)}</p>
                  <Drilldown drilldown={k.drilldown} />
                </article>
              )}
            </For>
          </section>

          <table class="executive-branches">
            <thead>
              <tr>
                <th>{t('colBranch')}</th>
                <th>{t('colVolume')}</th>
                <th>{t('colCash')}</th>
                <th>{t('colShifts')}</th>
                <th>{t('colSync')}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(r) => (
                  <tr data-branch={r.branchId}>
                    <td>{r.name}</td>
                    <td class="executive-money">{formatMoney(r.volumeThb)}</td>
                    <td class="executive-money">
                      <MetricValue metric={r.availableCash} money class="executive-cash" />
                      <span class="executive-transit">
                        {t('inTransit')} <MetricValue metric={r.inTransit} money class="executive-cash" />
                      </span>
                    </td>
                    <td class="executive-shifts">
                      <span>{t('shiftsOpen', { n: r.shifts.open })}</span>
                      <span>{t('shiftsVerified', { n: r.shifts.closedVerified })}</span>
                      <span>{t('shiftsUnverified', { n: r.shifts.closedUnverified })}</span>
                    </td>
                    <td>
                      <Time at={r.asOf} />
                    </td>
                    <td>
                      <Drilldown drilldown={r.drilldown} />
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>

          <section class="executive-alerts">
            <h3 class="executive-alerts-title">{t('alerts')}</h3>
            <Show when={alerts().length > 0} fallback={<p class="executive-no-alerts">{t('noAlerts')}</p>}>
              <ul class="executive-alert-list">
                <For each={alerts()}>
                  {(a) => (
                    <li class="executive-alert" data-alert-kind={a.kind}>
                      <span class="executive-alert-kind">
                        {ALERT_LABEL[a.kind] ? t(ALERT_LABEL[a.kind]!) : a.kind}
                      </span>
                      <span class="executive-alert-branch">{branchName(a.branchId)}</span>
                      <Show when={a.since}>{(since) => <time datetime={since()}>{bangkok(since())}</time>}</Show>
                      <span class="executive-alert-detail">{a.detail}</span>
                      <Drilldown drilldown={a.drilldown} />
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </section>
        </Show>
      </Show>
    </div>
  );
}
