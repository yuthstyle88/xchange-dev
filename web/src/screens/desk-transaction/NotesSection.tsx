// F50 (docs/design/backdated-keying.md §16.5): the transaction's note on the desk detail.
import { Show, type JSX } from 'solid-js';
import { t } from './texts';

/** F50b: when `notes` has text, a section headed by the screen's own word for "note" showing it as written. */
export function NotesSection(props: { notes: string | null | undefined }): JSX.Element {
  return (
    <Show when={props.notes?.trim()}>
      <section class="xc-dtx-section" data-section="notes">
        <h2 class="xc-dtx-heading">{t('notes')}</h2>
        <div data-testid="tx-notes" style="white-space: pre-wrap;">
          {props.notes}
        </div>
      </section>
    </Show>
  );
}
