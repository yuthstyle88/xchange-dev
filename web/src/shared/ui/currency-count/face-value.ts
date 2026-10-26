function groupInt(intPart: string): string {
  const isNegative = intPart.startsWith('-');
  const unsignedInt = isNegative ? intPart.slice(1) : intPart;
  const grouped = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${isNegative ? '-' : ''}${grouped}`;
}

export function formatFaceValue(wire: string): string {
  if (!/^-?\d+(\.\d+)?$/.test(wire)) return wire;
  const [intPart, decPart] = wire.split('.');
  const grouped = groupInt(intPart!);
  if (!decPart) return grouped;
  const dec = decPart.replace(/0+$/, '');
  return dec === '' ? grouped : `${grouped}.${dec.length < 2 ? dec.padEnd(2, '0') : dec}`;
}

export function formatAmount(wire: string): string {
  if (!/^-?\d+(\.\d+)?$/.test(wire)) return wire;
  const [intPart, decPart] = wire.split('.');
  const grouped = groupInt(intPart!);
  const dec = (decPart ?? '').replace(/0+$/, '');
  return `${grouped}.${dec.length < 2 ? dec.padEnd(2, '0') : dec}`;
}
