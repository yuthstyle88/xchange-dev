// Builds api/static/xc-forms.js from src/ (issue #378).
//
//   node build.mjs           write the script
//   node build.mjs --check   exit 1 when the committed script differs from a fresh build
//
// The Rust server embeds the committed file (include_str!, api/src/presentation/pages.rs), so a
// cargo build never needs Node. CI runs --check so the file cannot drift from its source.

import { build } from 'esbuild';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const HERE = fileURLToPath(new URL('.', import.meta.url));
const OUT = fileURLToPath(new URL('../../static/xc-forms.js', import.meta.url));
const HEADER =
  '// GENERATED from api/web/forms/src by api/web/forms/build.mjs — do not edit.\n' +
  '// Rebuild: npm --prefix api/web/forms ci && npm --prefix api/web/forms run build\n';

const result = await build({
  // esbuild names each module in a comment relative to this directory; pinning it makes the
  // output the same whichever directory the build is started from.
  absWorkingDir: HERE,
  entryPoints: ['src/index.ts'],
  bundle: true,
  format: 'iife',
  target: 'es2020',
  platform: 'browser',
  charset: 'utf8',
  legalComments: 'none',
  write: false,
  logLevel: 'error',
});
const fresh = HEADER + result.outputFiles[0].text;

if (process.argv.includes('--check')) {
  let committed = '';
  try {
    committed = readFileSync(OUT, 'utf8');
  } catch {}
  if (committed !== fresh) {
    console.error('api/static/xc-forms.js is not the build of api/web/forms/src — run: npm --prefix api/web/forms run build');
    process.exit(1);
  }
  console.log('api/static/xc-forms.js matches its source');
} else {
  writeFileSync(OUT, fresh);
  console.log(`wrote ${OUT} (${fresh.length} bytes)`);
}
