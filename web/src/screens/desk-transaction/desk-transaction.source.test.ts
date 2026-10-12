// TD-3 #1291: source rules of `web/src/screens/desk-transaction/**` (design D3, D8, D10, §4.5 last rows).
// Written before the implementation by the up-front test writer (claude/709ec749). Do not edit, delete or weaken.
//
// - No arithmetic or float conversion on the server's figures: no `Number(`, `parseFloat`, `toFixed`, `Math.` (D3).
// - No role name: permissions only, through S-3's `can()` (D8).
// - `can` is imported from S-3's `staff/parts/can.ts`, not copied (D8).
// - TH and EN only: `i18n.ts` exports `th` and `en` with the same keys, and no `zh` (D10).
// - The page is `xc-path` `/desk/transactions/{id}`, `xc-gate` `branch` (D1).
import { beforeAll, describe, expect, it } from 'vitest';
import * as texts from './i18n';

interface NodeFs {
  readdirSync(dir: string, opts: { withFileTypes: true }): { name: string; isDirectory(): boolean }[];
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  join(...s: string[]): string;
  dirname(p: string): string;
  resolve(...s: string[]): string;
}

let fs: NodeFs;
let path: NodePath;
let DIR: string;
let WEB: string;

const fsName = 'node:fs';
const pathName = 'node:path';

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  DIR = path.dirname(new URL(import.meta.url).pathname);
  WEB = path.resolve(DIR, '../../..');
});

function sources(): { file: string; text: string }[] {
  const out: { file: string; text: string }[] = [];
  const walk = (d: string) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/\.(ts|tsx)$/.test(e.name) && !/\.test\.(ts|tsx)$/.test(e.name)) {
        out.push({ file: p, text: fs.readFileSync(p, 'utf-8') });
      }
    }
  };
  walk(DIR);
  return out;
}

describe('desk-transaction source rules', () => {
  it('has no Number(, parseFloat, toFixed or Math. on any figure (D3)', () => {
    const hits = sources().flatMap(({ file, text }) =>
      [/\bNumber\(/, /\bparseFloat\b/, /\.toFixed\(/, /\bMath\./]
        .filter((re) => re.test(text))
        .map((re) => `${file}: ${re}`),
    );
    expect(hits).toEqual([]);
  });

  it('names no role (D8)', () => {
    const roles = /\b(TELLER|BRANCH_MANAGER|FINANCE|FRONT|BACKEND|MANAGER|EXECUTIVE|OWNER)\b/;
    const hits = sources()
      .filter(({ text }) => roles.test(text))
      .map(({ file, text }) => `${file}: ${text.match(roles)![0]}`);
    expect(hits).toEqual([]);
  });

  it("imports can() from S-3's staff/parts/can, and defines no can of its own (D8)", () => {
    const all = sources();
    expect(all.some(({ text }) => /from ['"]\.\.\/staff\/parts\/can['"]/.test(text)), 'imports staff/parts/can').toBe(
      true,
    );
    expect(all.filter(({ text }) => /function\s+can\s*\(/.test(text)).map((s) => s.file)).toEqual([]);
  });

  it('i18n exports th and en with the same keys, and no zh (D10)', () => {
    const t = texts as unknown as Record<string, Record<string, string> | undefined>;
    expect(t['zh']).toBeUndefined();
    expect(Object.keys(t['th'] ?? {}).sort()).toEqual(Object.keys(t['en'] ?? {}).sort());
  });

  it('the page is /desk/transactions/{id}, gated branch, loading this folder (D1)', () => {
    const html = fs.readFileSync(path.join(WEB, 'pages/desk-transaction.html'), 'utf-8');
    expect(html).toMatch(/<meta name="xc-path" content="\/desk\/transactions\/\{id\}">/);
    expect(html).toMatch(/<meta name="xc-gate" content="branch">/);
    expect(html).toContain('src="/src/screens/desk-transaction/main.tsx"');
  });
});
