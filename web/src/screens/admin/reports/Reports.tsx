import {
  createEffect,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type {
  AdminReportsData,
  AdminReportsFilter,
  ReportChannel,
  ReportEmployeeGroup,
} from '../../../shared/api/generated/admin_reports';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, screenT } from '../../../shared/i18n';
import { Card, CardBody, CardGrid, CardHead } from '../../../shared/ui/Card';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { StatCard } from '../../../shared/ui/StatCard';
import type { AdminMenuProps } from '../menus';
import { downloadCsv, walkPages } from '../parts/csv';
import { DataTable } from '../parts/DataTable';
import { closeDialog, openDialog } from '../parts/Dialog';
import { refreshTick } from '../parts/refresh';
import { toast } from '../parts/toast';
import { methodName, statusName, typeName } from '../parts/transaction';
import { TransactionDetail } from '../parts/TransactionDetail';
import { formatBangkokTime } from '../parts/TransactionTable';
import * as table from './i18n';
import './reports.css';

const t = screenT('admin.reports', table);

const CSV_HEADERS = [
  'Branch ID',
  'Branch',
  'Mode',
  'Document Ref',
  'Date',
  'Type',
  'Currency',
  'Amount',
  'Rate',
  'Principal THB',
  'Fee THB',
  'Bank Fee THB',
  'Settlement THB',
  'Method',
  'Status',
  'Shift ID',
  'Reference Rate',
  'Reference Value THB',
  'Reference Spread THB',
  'Reference Spread %',
];

const EMPLOYEE_CSV_HEADERS = [
  'พนักงาน',
  'เลขรายการ',
  'วันที่',
  'สถานะ',
  'ช่องทาง',
  'ประเภท',
  'สกุลเงิน',
  'จำนวน',
  'รับเข้า THB',
  'จ่ายออก THB',
  'ค่าบริการ THB',
  'ค่าธรรมเนียมธนาคาร THB',
];

function employeeMethodName(method: string): string {
  if (method === 'PromptPay') return 'PromptPay';
  if (method === 'Alipay') return 'Alipay';
  return methodName(method);
}

export function Reports(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [activeTab, setActiveTab] = createSignal<number>(0);
  const [data, setData] = createSignal<AdminReportsData | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);
  const [selectedStaff, setSelectedStaff] = createSignal<string>('');
  const [searchStaff, setSearchStaff] = createSignal<string>('');
  const [employeeTxMap, setEmployeeTxMap] = createSignal<Record<string, TransactionRow[]>>({});

  let branchesLoaded = false;
  let lastKey = '';
  let lastVersion: number | undefined = undefined;
  const tabButtons: (HTMLButtonElement | undefined)[] = [];

  const branchName = (branchId: string) =>
    branches().find((b) => b.id === branchId)?.name ?? branchId;

  async function readBranches() {
    try {
      const res = await api.view<AdminBranchesData>('admin_branches');
      if (res.data) {
        setBranches(res.data.branches);
      }
    } catch {
      // ignore
    }
  }

  async function readReports(isRefresh: boolean) {
    const from = props.period().from;
    const to = props.period().to;
    if (from && to && from > to) return;

    const curBranch = props.branch();
    const curStaff = selectedStaff();
    const curStaffQ = searchStaff();

    const filterObj: AdminReportsFilter = {
      from,
      to,
      staff: curStaff,
      staffQ: curStaffQ,
    };

    try {
      const res = await api.view<AdminReportsData, AdminReportsFilter>('admin_reports', {
        branch: curBranch,
        since: isRefresh ? lastVersion : undefined,
        filter: filterObj,
      });
      lastVersion = res.version;
      if (res.changed && res.data) {
        setData(res.data);
        setError(null);
      }
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setError(errorText(err.code, err.message));
      } else {
        setError(String(err));
      }
    }
  }

  async function loadEmployeeTx(staff: string) {
    try {
      const res = await api.view<TransactionRow[]>('admin_transactions', {
        branch: props.branch(),
        filter: {
          from: props.period().from || undefined,
          to: props.period().to || undefined,
          staff,
        },
      });
      if (res.data) {
        setEmployeeTxMap((prev) => ({ ...prev, [staff]: res.data! }));
      }
    } catch {
      // ignore
    }
  }

  createEffect(() => {
    if (!props.shown()) return;
    refreshTick();

    const from = props.period().from;
    const to = props.period().to;
    if (from && to && from > to) return;

    const branch = props.branch();
    const staff = selectedStaff();
    const staffQ = searchStaff();
    const currentKey = `${branch}|${from}|${to}|${staff}|${staffQ}`;

    if (!branchesLoaded) {
      branchesLoaded = true;
      readBranches();
    }

    const isRefresh = currentKey === lastKey;
    lastKey = currentKey;

    readReports(isRefresh);
  });

  createEffect(() => {
    const staff = selectedStaff();
    if (staff && props.shown()) {
      loadEmployeeTx(staff);
    }
  });

  function onTabKeyDown(e: KeyboardEvent, index: number) {
    let next: number | undefined = undefined;
    if (e.key === 'ArrowRight') next = (index + 1) % 3;
    if (e.key === 'ArrowLeft') next = (index + 3 - 1) % 3;
    if (e.key === 'Home') next = 0;
    if (e.key === 'End') next = 2;
    if (next !== undefined) {
      e.preventDefault();
      setActiveTab(next);
      tabButtons[next]?.click();
      tabButtons[next]?.focus();
    }
  }

  async function handleExport() {
    const filter = {
      from: props.period().from || undefined,
      to: props.period().to || undefined,
    };
    const rows = await walkPages<TransactionRow>(api, 'admin_transactions', {
      branch: props.branch(),
      filter,
    });
    if (rows.length === 0) {
      toast(t('noTransactionsToExport'));
      return;
    }

    const bName = branchName(props.branch());
    const safeBranchName = bName.replace(/[^A-Za-z0-9_-]/g, '_');
    const today = new Date().toLocaleDateString('en-CA', { timeZone: 'Asia/Bangkok' });
    const fileName = `XCHANGE-LIVE-${safeBranchName}-${today}.csv`;

    const reversed = [...rows].reverse();
    const rowData = reversed.map((r) => [
      props.branch(),
      bName,
      'LIVE',
      r.id,
      r.date,
      r.type,
      r.currency,
      r.amount,
      r.rate,
      r.principal,
      r.fee,
      r.bankFee,
      r.settlement,
      r.method,
      r.status,
      r.shiftId ?? '',
      r.referenceRate ?? '',
      r.referenceValue ?? '',
      r.referenceSpread ?? '',
      r.referencePercent ?? '',
    ]);

    downloadCsv(fileName, CSV_HEADERS, rowData);
  }

  async function handleEmployeeCsv() {
    const filter = {
      from: props.period().from || undefined,
      to: props.period().to || undefined,
      staff: selectedStaff() || undefined,
      staffQ: searchStaff() || undefined,
    };
    const rows = await walkPages<TransactionRow>(api, 'admin_transactions', {
      branch: props.branch(),
      filter,
    });

    const fileName = `XC-Employees-${props.period().from}-${props.period().to}.csv`;
    if (rows.length === 0) {
      downloadCsv(fileName, EMPLOYEE_CSV_HEADERS, []);
      return;
    }

    const reversed = [...rows].reverse();
    const rowData = reversed.map((r) => [
      r.staff,
      r.id,
      r.date,
      r.status,
      employeeMethodName(r.method),
      r.type,
      r.currency,
      r.amount,
      r.status === 'COMPLETED' && r.type === 'SELL' ? r.settlement : '0',
      r.status === 'COMPLETED' && r.type === 'BUY' ? r.settlement : '0',
      r.status === 'COMPLETED' ? r.fee : '0',
      r.status === 'COMPLETED' ? r.bankFee : '0',
    ]);

    downloadCsv(fileName, EMPLOYEE_CSV_HEADERS, rowData);
  }

  function openTxDetail(row: TransactionRow) {
    openDialog(row.id, () => (
      <TransactionDetail
        api={api}
        branch={props.branch()}
        row={row}
        onClose={closeDialog}
      />
    ));
  }

  const allEmployeeNames = () => data()?.byEmployee.map((g) => g.staff) ?? [];

  const shownEmployees = () => {
    const list = data()?.byEmployee ?? [];
    const s = selectedStaff();
    const q = searchStaff().toLowerCase();
    return list.filter((g) => {
      if (s && g.staff !== s) return false;
      if (q && !g.staff.toLowerCase().includes(q)) return false;
      return true;
    });
  };

  const tabsDefs = [
    { label: () => t('tabPayment') },
    { label: () => t('tabCurrencies') },
    { label: () => t('tabEmployees') },
  ];

  return (
    <div class="reports-screen">
      <Show when={error()}>
        <div role="alert" class="error">{error()}</div>
      </Show>

      <div class="reports-toolbar">
        <button
          type="button"
          class="secondary"
          id="export"
          hidden={activeTab() === 2}
          onClick={handleExport}
        >
          {t('exportCsv')}
        </button>
      </div>

      <div role="tablist" class="admin-tabs" aria-label={t('tablistAria')}>
        <For each={tabsDefs}>
          {(tabDef, i) => (
            <button
              ref={(el) => (tabButtons[i()] = el)}
              type="button"
              role="tab"
              id={`workspace-reports-${i()}-tab`}
              aria-controls={`workspace-reports-${i()}`}
              aria-selected={activeTab() === i() ? 'true' : 'false'}
              tabIndex={activeTab() === i() ? 0 : -1}
              onClick={() => setActiveTab(i())}
              onKeyDown={(e) => onTabKeyDown(e, i())}
            >
              {tabDef.label()}
            </button>
          )}
        </For>
      </div>

      {/* Tab 0: Payment channels */}
      <div
        role="tabpanel"
        id="workspace-reports-0"
        aria-labelledby="workspace-reports-0-tab"
        hidden={activeTab() !== 0}
      >
        <CardGrid columns={4}>
          <StatCard
            label={t('statReceivedLabel')}
            value={data()?.stats.received ?? '0.00'}
            note={t('statReceivedNote')}
          />
          <StatCard
            label={t('statPaidLabel')}
            value={data()?.stats.paid ?? '0.00'}
            note={t('statPaidNote')}
          />
          <StatCard
            label={t('statFeeLabel')}
            value={data()?.stats.fees ?? '0.00'}
            note={t('statFeeNote')}
          />
          <StatCard
            label={t('statBankFeeLabel')}
            value={data()?.stats.bankFees ?? '0.00'}
            note={t('statBankFeeNote')}
          />
        </CardGrid>

        <Card>
          <CardHead
            eyebrow={t('settlementEyebrow')}
            title={t('settlementHeading')}
            aside={<span class="unit">THB</span>}
          />
          <CardBody flush>
            <div class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>{t('colChannel')}</th>
                    <th class="num">{t('colCount')}</th>
                    <th class="num">{t('colReceived')}</th>
                    <th class="num">{t('colPaid')}</th>
                    <th class="num">{t('colNet')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={data()?.byChannel ?? []}>
                    {(ch: ReportChannel) => (
                      <tr>
                        <td>{methodName(ch.method)}</td>
                        <td class="num">{String(ch.count)}</td>
                        <td class="num">{formatMoney(ch.received)}</td>
                        <td class="num">{formatMoney(ch.paid)}</td>
                        <td class="num">{formatMoney(ch.net)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </CardBody>
        </Card>
        <p class="footnote">{t('footnote')}</p>
      </div>

      {/* Tab 1: Currencies and spreads */}
      <div
        role="tabpanel"
        id="workspace-reports-1"
        aria-labelledby="workspace-reports-1-tab"
        hidden={activeTab() !== 1}
      >
        <Card>
          <CardHead
            eyebrow={t('currenciesEyebrow')}
            title={t('currenciesHeading')}
          />
          <CardBody flush>
            <div class="table-scroll">
              <DataTable
                headers={[
                  { text: t('colCurrency'), numeric: false },
                  { text: t('colBuyUnits'), numeric: true },
                  { text: t('colSellUnits'), numeric: true },
                  { text: t('colPrincipalThb'), numeric: true },
                  { text: t('colFeeThb'), numeric: true },
                ]}
                rowCount={data()?.byCurrency.length ?? 0}
              >
                <For each={data()?.byCurrency ?? []}>
                  {(c) => (
                    <tr>
                      <td><strong>{c.currency}</strong></td>
                      <td class="num">{formatMoney(c.buyAmount)}</td>
                      <td class="num">{formatMoney(c.sellAmount)}</td>
                      <td class="num">{formatMoney(c.principal)}</td>
                      <td class="num">{formatMoney(c.fees)}</td>
                    </tr>
                  )}
                </For>
              </DataTable>
            </div>
          </CardBody>
          <div id="report-reference" class="stock-banner">
            <strong>
              {t('referenceSpreadPrefix')}
              <Show
                when={(data()?.reference.referenced ?? 0) > 0}
                fallback={t('noReferenceRate')}
              >
                {formatMoney(data()?.reference.spread ?? '0.00')} THB
                <Show when={data()?.reference.percent}>
                  {' '}({data()?.reference.percent}%)
                </Show>
              </Show>
            </strong>
            <p>
              {data()?.reference.referenced ?? 0} {t('referenceCounts', {
                referenced: data()?.reference.referenced ?? 0,
                unreferenced: data()?.reference.unreferenced ?? 0,
                note: t('referenceNote'),
              })}
            </p>
          </div>
        </Card>
      </div>

      {/* Tab 2: Employee summary */}
      <div
        role="tabpanel"
        id="workspace-reports-2"
        aria-labelledby="workspace-reports-2-tab"
        hidden={activeTab() !== 2}
      >
        <Card>
          <div id="employee-summary">
            <CardHead
              title={t('employeeSummaryHeading')}
              aside={
                <button
                  type="button"
                  class="secondary"
                  id="employee-csv"
                  onClick={handleEmployeeCsv}
                >
                  {t('downloadCsv')}
                </button>
              }
            />
            <p class="unit">{t('employeeSummaryNote')}</p>

          <div style="display: flex; gap: 12px; flex-wrap: wrap; margin: 18px 0;">
            <label>
              {t('employeeLabel')}{' '}
              <select
                id="employee-select"
                value={selectedStaff()}
                onChange={(e) => setSelectedStaff(e.currentTarget.value)}
              >
                <option value="">{t('allStaff')}</option>
                <For each={allEmployeeNames()}>
                  {(staff) => (
                    <option value={staff} selected={staff === selectedStaff()}>
                      {staff}
                    </option>
                  )}
                </For>
              </select>
            </label>
            <label>
              {t('searchEmployee')}{' '}
              <input
                id="employee-search"
                type="search"
                placeholder={t('staffSearchPlaceholder')}
                value={searchStaff()}
                onInput={(e) => setSearchStaff(e.currentTarget.value)}
              />
            </label>
          </div>

          <p>
            <strong>
              {t('totalsLine', {
                people: data()?.employeeTotals.people ?? 0,
                completed: data()?.employeeTotals.completed ?? 0,
                received: formatMoney(data()?.employeeTotals.received ?? '0.00'),
                paid: formatMoney(data()?.employeeTotals.paid ?? '0.00'),
                net: formatMoney(data()?.employeeTotals.net ?? '0.00'),
              })}
            </strong>
          </p>

          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>{t('employeeLabel')}</th>
                  <th class="num">{t('colAll')}</th>
                  <th class="num">{t('colCompleted')}</th>
                  <th class="num">{t('colPending')}</th>
                  <th class="num">{t('colCancelled')}</th>
                  <th class="num">{t('colReceivedThb')}</th>
                  <th class="num">{t('colPaidThb')}</th>
                  <th class="num">{t('colNetThb')}</th>
                  <th class="num">{t('colServiceFee')}</th>
                  <th class="num">{t('colBankFee')}</th>
                </tr>
              </thead>
              <tbody>
                <For each={shownEmployees()}>
                  {(g: ReportEmployeeGroup) => (
                    <tr>
                      <td>{g.staff}</td>
                      <td class="num">{String(g.count)}</td>
                      <td class="num">{String(g.completed)}</td>
                      <td class="num">{String(g.pending)}</td>
                      <td class="num">{String(g.cancelled)}</td>
                      <td class="num">{formatMoney(g.received)}</td>
                      <td class="num">{formatMoney(g.paid)}</td>
                      <td class="num">{formatMoney(g.net)}</td>
                      <td class="num">{formatMoney(g.fees)}</td>
                      <td class="num">{formatMoney(g.bankFees)}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>

          <For each={shownEmployees()}>
            {(g: ReportEmployeeGroup) => (
              <details
                open={Boolean(selectedStaff())}
                style="border-top: 1px solid #e5dfe4; padding: 18px 0;"
                onToggle={(e) => {
                  if (e.currentTarget.open) {
                    loadEmployeeTx(g.staff);
                  }
                }}
              >
                <summary style="cursor: pointer; color: #a5003c; font-weight: bold;">
                  {t('employeeSectionSummary', { staff: g.staff, count: g.count })}
                </summary>

                <h3>{t('methodsHeading')}</h3>
                <div class="table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>{t('colChannel')}</th>
                        <th class="num">{t('colCompleted')}</th>
                        <th class="num">{t('colReceivedThb')}</th>
                        <th class="num">{t('colPaidThb')}</th>
                        <th class="num">{t('colNetThb')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={g.methods}>
                        {(m) => (
                          <tr>
                            <td>{employeeMethodName(m.method)}</td>
                            <td class="num">{String(m.count)}</td>
                            <td class="num">{formatMoney(m.received)}</td>
                            <td class="num">{formatMoney(m.paid)}</td>
                            <td class="num">{formatMoney(m.net)}</td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>

                <h3>{t('employeeCurrenciesHeading')}</h3>
                <div class="table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>{t('colCurrency')}</th>
                        <th class="num">{t('colUnitsReceived')}</th>
                        <th class="num">{t('colUnitsPaid')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={g.currencies}>
                        {(c) => (
                          <tr>
                            <td>{c.currency}</td>
                            <td class="num">{formatMoney(c.received)}</td>
                            <td class="num">{formatMoney(c.paid)}</td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>

                <h3>{t('transactionsHeading')}</h3>
                <div class="table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>{t('colTxTime')}</th>
                        <th>{t('colStatus')}</th>
                        <th>{t('colChannel')}</th>
                        <th>{t('colType')}</th>
                        <th class="num">{t('colAmount')}</th>
                        <th class="num">{t('colReceivedThb')}</th>
                        <th class="num">{t('colPaidThb')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={employeeTxMap()[g.staff] ?? []}>
                        {(row: TransactionRow) => (
                          <tr>
                            <td>
                              <div>
                                <button
                                  type="button"
                                  class="ref link-button"
                                  data-employee-ref={row.id}
                                  onClick={() => openTxDetail(row)}
                                >
                                  {row.id}
                                </button>
                              </div>
                              <div class="small">
                                {row.date} {formatBangkokTime(row.created)}
                              </div>
                            </td>
                            <td>{statusName(row.status)}</td>
                            <td>{employeeMethodName(row.method)}</td>
                            <td>{typeName(row.type)}</td>
                            <td class="num">
                              {formatMoney(row.amount)} {row.currency}
                            </td>
                            <td class="num">
                              {row.status === 'COMPLETED' && row.type === 'SELL'
                                ? formatMoney(row.settlement)
                                : '—'}
                            </td>
                            <td class="num">
                              {row.status === 'COMPLETED' && row.type === 'BUY'
                                ? formatMoney(row.settlement)
                                : '—'}
                            </td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>
              </details>
            )}
          </For>
          </div>
        </Card>
      </div>
    </div>
  );
}
