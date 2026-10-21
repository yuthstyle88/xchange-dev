// QW-1 (kiosk-shared-phone §5r F13, R1): parses a customer's request QR URL (`<origin>/q#<token>`).
export function parseRequestQr(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
  if (url.pathname !== '/q' || url.search !== '') return null;
  const token = url.hash.startsWith('#') ? url.hash.slice(1) : '';
  return token.length > 0 ? token : null;
}
