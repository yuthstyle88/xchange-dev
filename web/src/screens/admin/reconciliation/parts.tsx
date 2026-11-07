// Shared pieces of F01's sheet (#1068), split out of Reconciliation.tsx so each part of the sheet
// can live in its own file (FE-admin skeleton, #564).
import { Show, type JSX } from 'solid-js';
import type { Part } from '../../../shared/api/generated/finance_reconciliation';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

const t = screenT('admin.reconciliation', table);

export { formatBangkokDateTime as stamp } from '../../../shared/format/bangkok-time';

export type PartName = 'transactions' | 'counts' | 'transfers' | 'exceptions' | 'integrity' | 'reversalsPending';

/** One part of the sheet: its heading, then its body, or what it waits for when unavailable. */
export function PartBox<T>(props: {
  name: PartName;
  part: Part<T>;
  children: (data: T) => JSX.Element;
}): JSX.Element {
  return (
    <section
      class="card reconciliation-part"
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

export function Empty(): JSX.Element {
  return <p class="reconciliation-none">{t('none')}</p>;
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function Who(props: { id: string }): JSX.Element {
  return <span title={props.id}>{UUID_RE.test(props.id) ? props.id.slice(0, 8) : props.id}</span>;
}
