import type { JSX } from 'solid-js';

export function formatMoney(value: string): string {
  const isDecimal = /^-?\d+(\.\d+)?$/.test(value);
  if (!isDecimal) {
    return value;
  }

  const [intPart, decPart] = value.split('.');
  const isNegative = intPart!.startsWith('-');
  const unsignedInt = isNegative ? intPart!.slice(1) : intPart!;
  const groupedInt = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${isNegative ? '-' : ''}${groupedInt}${decPart !== undefined ? `.${decPart}` : ''}`;
}

export function MoneyText(props: { value: string; currency: string }): JSX.Element {
  const formatted = () => formatMoney(props.value);

  return <span>{formatted()} {props.currency}</span>;
}
