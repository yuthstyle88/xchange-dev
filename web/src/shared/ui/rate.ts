/**
 * RE-5 (#602): The one rate formatter per design §3.3.
 * Formats a rate string with 2 to 8 decimal places and optional unit suffix.
 * Never uses JS float/number parsing to preserve exact digit precision.
 */
export function formatRate(rate: string, unitSize: number | string, code: string): string {
  const parts = rate.split('.');
  const intPart = parts[0] || '0';
  let fracPart = parts[1] || '';

  // Trim trailing zeros down to at least 2 decimal places
  while (fracPart.length > 2 && fracPart.endsWith('0')) {
    fracPart = fracPart.slice(0, -1);
  }

  // Pad to at least 2 decimal places
  while (fracPart.length < 2) {
    fracPart += '0';
  }

  const formattedNumber = `${intPart}.${fracPart}`;

  // Unit size handling without number parsing
  const unitStr = `${unitSize}`.replace(/\.0+$/, '');
  if (unitStr === '1' || unitStr === '0') {
    return formattedNumber;
  }

  return `${formattedNumber} / ${unitStr} ${code}`;
}
