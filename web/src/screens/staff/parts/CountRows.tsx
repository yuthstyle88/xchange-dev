import { For, type JSX } from 'solid-js';

export interface DenominationRow {
  faceValueId: number;
  label?: string;
  value: number;
}

export interface CountRowsProps {
  denominations: readonly DenominationRow[];
  quantities: Record<number, string>;
  onChange: (faceValueId: number, quantity: string) => void;
}

export function CountRows(props: CountRowsProps): JSX.Element {
  return (
    <div class="xc-count-rows">
      <For each={props.denominations}>
        {(d) => (
          <div class="xc-count-row">
            <span>{d.label ?? String(d.value)}</span>
            <input
              type="number"
              min="0"
              step="1"
              value={props.quantities[d.faceValueId] ?? ''}
              onInput={(e) => props.onChange(d.faceValueId, e.currentTarget.value)}
            />
          </div>
        )}
      </For>
    </div>
  );
}
