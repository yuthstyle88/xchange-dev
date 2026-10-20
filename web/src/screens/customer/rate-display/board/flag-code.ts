// P01 v2 flags (rate-display.md §9.7): a currency → the URL of its vendored flag, or null.

// Vite 8 inlines <4 KiB SVGs on `query: '?url'` unless `&no-inline` is passed;
// strip the dev `?no-inline` suffix so resolved URLs end with `<cc>.svg`.
const FLAG_URLS = import.meta.glob<string>('./flags/*.svg', {
  eager: true,
  query: '?url&no-inline',
  import: 'default',
});

/** `USD` → the vendored `us.svg` URL; codes with no country flag (`XAU`, `XAG`, `X*`) or no vendored file → null. */
export function flagUrl(currency: string): string | null {
  if (!currency || currency.length < 2) return null;
  if (currency.toUpperCase().startsWith('X')) return null;
  const cc = currency.slice(0, 2).toLowerCase();
  const url = FLAG_URLS[`./flags/${cc}.svg`];
  return url ? url.replace(/\?no-inline$/, '') : null;
}


