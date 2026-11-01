// TD-3 #1291: one label and its value. D3 means no formatting of figures; times are formatted to ICT by the callers (W1).
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
