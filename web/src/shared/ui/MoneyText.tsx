import type { JSX } from 'solid-js';

export function MoneyText(props: { value: string; currency: string }): JSX.Element {
  const isDecimal = /^-?\d+(\.\d+)?$/.test(props.value);
  if (!isDecimal) {
    return <span>{props.value} {props.currency}</span>;
  }

  const [intPart, decPart] = props.value.split('.');
  const isNegative = intPart!.startsWith('-');
  const unsignedInt = isNegative ? intPart!.slice(1) : intPart!;
  const groupedInt = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const formatted = `${isNegative ? '-' : ''}${groupedInt}${decPart !== undefined ? `.${decPart}` : ''}`;

  return <span>{formatted} {props.currency}</span>;
}
