// Records what a browser's Intl answers for the kiosk's languages, so the
// installed app (which has no Intl) shows the same names as the web build.
//
//   node tool/record_display_names.mjs            # writes assets/l10n/display_names.json
//   CHROME=/path/to/chrome node tool/record_display_names.mjs
//
// Same questions, asked the same way, as BrowserDisplayNames
// (lib/core/platform/display_names_web.dart): names through
// Intl.DisplayNames([lang, 'en']), symbols through Intl.NumberFormat('en-US',
// narrowSymbol); an answer equal to the code is "no name" and is left out.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const langs = JSON.parse(readFileSync(join(app, 'assets/l10n/kiosk.json'), 'utf8'))
  .langs.map((l) => l.lang ?? l.code);
// The four CJK fonts are subsets cut for exactly the names in
// cjk_currency_names.json, so those names are copied, not asked again: a newer
// browser words some of them differently and the fonts would lack the glyphs.
const cjk = JSON.parse(readFileSync(join(app, 'tool/cjk_currency_names.json'), 'utf8')).currencies;
const codes = Object.keys(cjk.zh.names).sort();

function record(langs, codes) {
  const name = (lang, type, code) => {
    try {
      const n = new Intl.DisplayNames([lang, 'en'], { type }).of(code);
      return n == null || n === code ? null : n;
    } catch (_) {
      return null;
    }
  };
  const symbol = (code) => {
    try {
      const parts = new Intl.NumberFormat('en-US', {
        style: 'currency', currency: code, currencyDisplay: 'narrowSymbol',
      }).formatToParts(0);
      for (const p of parts) if (p.type === 'currency') return p.value;
    } catch (_) {}
    return code;
  };
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
  const regionCodes = [];
  for (const a of A) for (const b of A) if (name('en', 'region', a + b)) regionCodes.push(a + b);
  const out = { recordedWith: navigator.userAgent, symbols: {}, currencies: {}, regions: {} };
  for (const c of codes) { const s = symbol(c); if (s !== c) out.symbols[c] = s; }
  for (const lang of langs) {
    out.currencies[lang] = {};
    out.regions[lang] = {};
    for (const c of codes) { const n = name(lang, 'currency', c); if (n) out.currencies[lang][c] = n; }
    for (const r of regionCodes) { const n = name(lang, 'region', r); if (n) out.regions[lang][r] = n; }
  }
  return out;
}

const chrome = process.env.CHROME ??
  (process.platform === 'darwin'
    ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
    : 'google-chrome');
const dir = mkdtempSync(join(tmpdir(), 'display-names-'));
const page = join(dir, 'record.html');
writeFileSync(page, `<!doctype html><meta charset="utf-8"><pre id="out"></pre><script>
document.getElementById('out').textContent = btoa(unescape(encodeURIComponent(JSON.stringify(
  (${record.toString()})(${JSON.stringify(langs)}, ${JSON.stringify(codes)})))));
</script>`);
const dom = execFileSync(chrome, ['--headless=new', '--disable-gpu', '--dump-dom', pathToFileURL(page).href],
  { encoding: 'utf8', maxBuffer: 64 << 20 });
rmSync(dir, { recursive: true });
const m = /<pre id="out">([^<]+)<\/pre>/.exec(dom);
if (!m) throw new Error('the browser printed no result');
const result = JSON.parse(Buffer.from(m[1], 'base64').toString('utf8'));
for (const { lang, names } of Object.values(cjk)) {
  result.currencies[lang] = Object.fromEntries(
    Object.entries(names).filter(([code, name]) => name !== code).sort(([a], [b]) => (a < b ? -1 : 1)),
  );
}
const target = join(app, 'assets/l10n/display_names.json');
writeFileSync(target, JSON.stringify(result, null, 1) + '\n');
console.log(`${target}: ${langs.length} languages, ${codes.length} currency codes, ` +
  `${Object.keys(result.regions.en).length} regions, recorded with ${result.recordedWith}`);
