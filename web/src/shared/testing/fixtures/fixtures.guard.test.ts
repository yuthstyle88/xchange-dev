// Web test fixtures, guard 1 (docs/design/web-test-fixtures.md §4.2): every generated object type has
// a generated factory, and the committed fixture files are exactly what `npm run gen:fixtures`
// writes. A BE PR that regenerates `shared/api/generated/*.ts` and forgets `gen:fixtures` fails here,
// in its own CI.

import { beforeAll, describe, expect, it } from 'vitest';

interface ChildProcess {
  spawnSync(
    cmd: string,
    args: string[],
    opts: { cwd: string; encoding: string },
  ): { status: number | null; stdout: string; stderr: string };
}
interface NodePath {
  resolve(...segments: string[]): string;
  dirname(p: string): string;
}

const childProcessName = 'node:child_process';
const pathName = 'node:path';

let cp: ChildProcess;
let WEB_ROOT: string;

beforeAll(async () => {
  cp = (await import(/* @vite-ignore */ childProcessName)) as unknown as ChildProcess;
  const path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  WEB_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../../../..');
});

describe('web test fixtures (design §4.2)', () => {
  it('every generated object type has a factory, and no fixture file is missing, stale or orphaned', () => {
    const r = cp.spawnSync('node', ['scripts/gen-fixtures.mjs', '--check'], { cwd: WEB_ROOT, encoding: 'utf8' });
    expect(r.status, `${r.stdout}\n${r.stderr}`).toBe(0);
  });
});
