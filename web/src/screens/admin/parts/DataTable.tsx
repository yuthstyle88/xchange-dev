// V2-parts (#476): data table with empty state (ticket §7, design §12.2, P1)

import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { en, th } from './i18n';
import './parts.css';

export interface DataTableHeader {
  text: string;
  numeric?: boolean;
}

export interface DataTableProps {
  headers: DataTableHeader[];
  rowCount: number;
  empty?: string;
  emptyNote?: string;
  children?: JSX.Element;
}

export function DataTable(props: DataTableProps): JSX.Element {
  const t = screenT('admin.parts', { th, en });

  return (
    <Show
      when={props.rowCount > 0}
      fallback={
        <div class="data-table-empty">
          <p>{props.empty ?? t('emptyTransactions')}</p>
          <p class="empty-note">{props.emptyNote ?? t('emptyTransactionsNote')}</p>
        </div>
      }
    >
      <table class="data-table">
        <thead>
          <tr>
            <For each={props.headers}>
              {(h) => <th classList={{ num: !!h.numeric }}>{h.text}</th>}
            </For>
          </tr>
        </thead>
        <tbody>{props.children}</tbody>
      </table>
    </Show>
  );
}
