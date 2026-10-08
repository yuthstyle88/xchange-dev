// V2-parts (#476): hash query parameters and openMenu (ticket §7, design §12.14)

export function openMenu(id: string, params: Record<string, string>): void {
  const parts = Object.entries(params).map(
    ([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`,
  );
  const qs = parts.join('&');
  location.hash = qs ? `#${id}?${qs}` : `#${id}`;
}

export function hashParams(): URLSearchParams {
  const hash = typeof location !== 'undefined' ? location.hash : '';
  const qIndex = hash.indexOf('?');
  if (qIndex === -1) {
    return new URLSearchParams();
  }
  return new URLSearchParams(hash.slice(qIndex + 1));
}
