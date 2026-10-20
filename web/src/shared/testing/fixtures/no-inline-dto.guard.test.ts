// Web test fixtures, guard 2 (docs/design/web-test-fixtures.md §4.3): a test never writes a
// generated object type by hand. A test file fails when it
//   - imports from `shared/api/generated/` (it imports types and factories from
//     `shared/testing/fixtures` instead), or
//   - builds a generated object type by hand: a typed literal `: T = {` / `: T[] = [`, a function
//     whose declared return type is `T` / `T[]` (`): T {`, `): T => ({`), `as T`, or `satisfies T`.
//     A plain annotation such as a parameter `(r: T) =>` is allowed: it never breaks when the
//     back end adds a field.
// Rollout: the not-yet-migrated files are listed, one path (relative to web/src) per line, in
// `inline-dto-allowlist/<folder>.txt`, one file per screen folder. A migration ticket deletes its
// folder's file. The allowlist only shrinks: a listed file that is already clean also fails.

import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
  statSync(file: string): { isDirectory(): boolean };
}
interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
  relative(from: string, to: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let SRC: string;
let FIXTURES: string;
let ALLOWLIST_DIR: string;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  FIXTURES = path.dirname(new URL(import.meta.url).pathname);
  SRC = path.resolve(FIXTURES, '../../..');
  ALLOWLIST_DIR = path.join(FIXTURES, 'inline-dto-allowlist');
});

/** Generated object type names (`export type X = {` or `export type X<T> = {`). */
function generatedObjectNames(): Set<string> {
  const dir = path.join(SRC, 'shared/api/generated');
  const names = new Set<string>();
  for (const f of fs.readdirSync(dir)) {
    if (!f.endsWith('.ts')) continue;
    const src = fs.readFileSync(path.join(dir, f), 'utf8');
    for (const m of src.matchAll(/^export type (\w+)(?:<[^=]*>)? = \{/gm)) names.add(m[1]!);
  }
  return names;
}

function testFiles(dir: string, out: string[] = []): string[] {
  for (const name of fs.readdirSync(dir)) {
    const p = path.join(dir, name);
    if (fs.statSync(p).isDirectory()) {
      if (p === FIXTURES) continue;
      testFiles(p, out);
    } else if (/\.test\.tsx?$/.test(name)) {
      out.push(p);
    }
  }
  return out;
}

/** Comments and string contents removed, so a type name in prose or a string never counts. */
function code(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1')
    .replace(/`(?:\\[\s\S]|[^`\\])*`/g, '``')
    .replace(/'(?:\\.|[^'\\\n])*'/g, "''")
    .replace(/"(?:\\.|[^"\\\n])*"/g, '""');
}

/** Why `file` writes a generated object by hand, or [] when it does not. */
export function violations(src: string, names: Set<string>): string[] {
  const why: string[] = [];
  if (/from\s+['"][^'"]*shared\/api\/generated\//.test(src)) why.push('imports from shared/api/generated');
  const body = code(src);
  const forms: [RegExp, string][] = [
    [/:\s*([A-Z]\w*)(?:<[^=]*?>)?(?:\[\])?\s*=\s*[{[]/g, 'typed literal'],
    [/\)\s*:\s*([A-Z]\w*)(?:<[^{=]*?>)?(?:\[\])?\s*(?:\{|=>\s*[({[])/g, 'function returning'],
    [/\bas\s+([A-Z]\w*)\b/g, 'cast'],
    [/\bsatisfies\s+([A-Z]\w*)\b/g, 'satisfies'],
  ];
  for (const [re, form] of forms) {
    for (const m of body.matchAll(re)) {
      if (names.has(m[1]!)) why.push(`${form} of generated type ${m[1]}`);
    }
  }
  return [...new Set(why)];
}

function allowlisted(): Map<string, string> {
  const listed = new Map<string, string>();
  if (!fs.existsSync(ALLOWLIST_DIR)) return listed;
  for (const f of fs.readdirSync(ALLOWLIST_DIR)) {
    if (!f.endsWith('.txt')) continue;
    for (const line of fs.readFileSync(path.join(ALLOWLIST_DIR, f), 'utf8').split('\n')) {
      const p = line.trim();
      if (p && !p.startsWith('#')) listed.set(p, f);
    }
  }
  return listed;
}

describe('no inline generated objects in tests (design §4.3)', () => {
  it('no test file outside the allowlist imports or names a generated object type', () => {
    const names = generatedObjectNames();
    const listed = allowlisted();
    const offenders: string[] = [];
    for (const file of testFiles(SRC)) {
      const rel = path.relative(SRC, file);
      if (listed.has(rel)) continue;
      const why = violations(fs.readFileSync(file, 'utf8'), names);
      if (why.length) offenders.push(`${rel}: ${why.join('; ')}`);
    }
    expect(offenders, 'build these objects with shared/testing/fixtures factories').toEqual([]);
  });

  it('the allowlist only lists files that exist and still need migrating (it only shrinks)', () => {
    const names = generatedObjectNames();
    const stale: string[] = [];
    for (const [rel, list] of allowlisted()) {
      const file = path.join(SRC, rel);
      if (!fs.existsSync(file)) stale.push(`${list}: ${rel} does not exist`);
      else if (violations(fs.readFileSync(file, 'utf8'), names).length === 0) {
        stale.push(`${list}: ${rel} is already clean; delete its line`);
      }
    }
    expect(stale).toEqual([]);
  });

  it('the scanner itself: flags imports and hand-built objects; allows plain annotations, comments and strings', () => {
    const names = new Set(['AdminCashData']);
    const flagged = (s: string) => violations(s, names).length;
    expect(flagged("import type { X } from '../../shared/api/generated/admin_cash';")).toBe(1);
    expect(flagged('const d: AdminCashData = { a: 1 };')).toBe(1);
    expect(flagged('const rows: AdminCashData[] = [];')).toBe(1);
    expect(flagged('function mk(id: string): AdminCashData {\n  return { id };\n}')).toBe(1);
    expect(flagged('const mk = (): AdminCashData => ({ a: 1 });')).toBe(1);
    expect(flagged('const d = {} as AdminCashData;')).toBe(1);
    expect(flagged('const d = x satisfies AdminCashData;')).toBe(1);
    expect(flagged('rows.map((r: AdminCashData) => r.id);')).toBe(0);
    expect(flagged('// AdminCashData in prose\nconst s = "as AdminCashData";')).toBe(0);
    expect(flagged('const d = makeAdminCashData({});')).toBe(0);
  });
});
