// Shared pieces of F01's sheet (#1068), split out of Reconciliation.tsx so each part of the sheet
// can live in its own file (FE-admin skeleton, #564).
import { Show, type JSX } from 'solid-js';
import type { Part } from '../../../shared/api/generated/finance_reconciliation';
import { locale, screenT } from '../../../shared/i18n';
import * as table from './i18n';

const t = screenT('admin.reconciliation', table);

export function stamp(iso: string): string {
  return new Date(iso).toLocaleString(locale(), {
    timeZone: 'Asia/Bangkok',
  });
}

export type PartName = 'transactions' | 'counts' | 'transfers' | 'exceptions' | 'integrity' | 'reversalsPending';

/** One part of the sheet: its heading, then its body, or what it waits for when unavailable. */
export function PartBox<T>(props: {
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

export function Empty(): JSX.Element {
  return <p class="reconciliation-none">{t('none')}</p>;
}
