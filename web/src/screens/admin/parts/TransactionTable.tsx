// V2-parts (#476): transaction table component (ticket §7, design §12.2, P1)

import { For, type JSX } from 'solid-js';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { DataTable, type DataTableHeader } from './DataTable';
import { en, th } from './i18n';
import { methodName, statusName, typeName } from './transaction';
import './parts.css';

export interface TransactionTableProps {
  rows: TransactionRow[];
  onOpen: (id: string) => void;
}

export function formatBangkokTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString('en-GB', {
      timeZone: 'Asia/Bangkok',
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
  } catch {
    return '';
  }
}

export function TransactionTable(props: TransactionTableProps): JSX.Element {
  const t = screenT('admin.parts', { th, en });

  const headers = (): DataTableHeader[] => [
    { text: t('headerTxTime'), numeric: false },
    { text: t('headerCustomer'), numeric: false },
    { text: t('headerType'), numeric: false },
    { text: t('headerAmount'), numeric: true },
    { text: t('headerSettlement'), numeric: true },
    { text: t('headerStatus'), numeric: false },
  ];

  return (
    <DataTable headers={headers()} rowCount={props.rows.length}>
      <For each={props.rows}>
        {(row) => (
          <tr>
            <td>
              <div>
                <button
                  type="button"
                  class="link-button"
                  onClick={() => props.onOpen(row.id)}
                >
                  {row.id}
                </button>
              </div>
              <div class="date-time">
                {row.date} · {formatBangkokTime(row.created)}
              </div>
            </td>
            <td>
              <div>{row.customer}</div>
              <div class="method">{methodName(row.method)}</div>
            </td>
            <td>
              <span
                class="pill"
                classList={{
                  buy: row.type === 'BUY',
                  sell: row.type !== 'BUY',
                }}
              >
                {typeName(row.type)}
              </span>
            </td>
            <td class="num">
              <div>
                {formatMoney(row.amount)} {row.currency}
              </div>
              <div class="rate">@ {row.rate}</div>
            </td>
            <td class="num">
              <div>{formatMoney(row.settlement)}</div>
            </td>
            <td>
              <span
                class="pill"
                classList={{
                  cancel: row.status === 'CANCELLED',
                }}
              >
                {statusName(row.status)}
              </span>
            </td>
          </tr>
        )}
      </For>
    </DataTable>
  );
}
