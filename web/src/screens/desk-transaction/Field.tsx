// TD-3 #1291: one label and its value, as the server sent it (D3: no formatting, no arithmetic).
import { Show, type JSX } from 'solid-js';

export function Field(props: { label: string; value: string | null | undefined }): JSX.Element {
  return (
    <Show when={props.value !== null && props.value !== undefined && props.value !== ''}>
      <div class="xc-dtx-field">
        <dt>{props.label}</dt>
        <dd>{props.value}</dd>
      </div>
    </Show>
  );
}
