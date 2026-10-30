import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
  statSync(file: string): { isDirectory(): boolean; isFile(): boolean };
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
  basename(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let webSrcDir: string;
let e2eTestsDir: string;

interface SelectorUsage {
  file: string;
  line: number;
  value: string;
}

const ATTR_RE = /data-(?:action|testid)\s*=\s*(?:\\*["'])([a-z0-9-]+)(?:\\*["'])/g;
const GET_RE = /getByTestId\s*\(\s*["']([a-z0-9-]+)["']\s*\)/g;

function checkSelectors(content: string, filename: string, webSrc: string): SelectorUsage[] {
  const missing: SelectorUsage[] = [];
  const lines = content.split('\n');

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    let match: RegExpExecArray | null;

    ATTR_RE.lastIndex = 0;
    while ((match = ATTR_RE.exec(line)) !== null) {
      const val = match[1];
      if (!webSrc.includes(`"${val}"`) && !webSrc.includes(`'${val}'`)) {
        missing.push({ file: filename, line: i + 1, value: val });
      }
    }

    GET_RE.lastIndex = 0;
    while ((match = GET_RE.exec(line)) !== null) {
      const val = match[1];
      if (!webSrc.includes(`"${val}"`) && !webSrc.includes(`'${val}'`)) {
        missing.push({ file: filename, line: i + 1, value: val });
      }
    }
  }

  return missing;
}

function walkWebSrc(dir: string): string[] {
  const files: string[] = [];
  if (!fs.existsSync(dir)) return files;
  for (const entry of fs.readdirSync(dir)) {
    if (entry === 'testing') continue;
    const full = path.join(dir, entry);
    const stat = fs.statSync(full);
    if (stat.isDirectory()) {
      files.push(...walkWebSrc(full));
    } else if (stat.isFile()) {
      if (
        (entry.endsWith('.ts') || entry.endsWith('.tsx')) &&
        !entry.endsWith('.test.ts') &&
        !entry.endsWith('.test.tsx')
      ) {
        files.push(full);
      }
    }
  }
  return files;
}

function loadCombinedWebSrc(): string {
  const files = walkWebSrc(webSrcDir);
  return files.map((f) => fs.readFileSync(f, 'utf8')).join('\n');
}

describe('e2e selectors gate (#5712)', () => {
  beforeAll(async () => {
    fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
    const thisFile = new URL(import.meta.url).pathname;
    const thisDir = path.dirname(thisFile);
    const webRoot = path.resolve(thisDir, '../../..');
    webSrcDir = path.resolve(thisDir, '../..');
    e2eTestsDir = path.resolve(webRoot, '../qa/e2e/tests');
  });

  it('fails when an e2e selector names a value no web/src file contains', () => {
    const webSrc = loadCombinedWebSrc();
    const specFiles = fs
      .readdirSync(e2eTestsDir)
      .filter((f) => f.endsWith('.ts'))
      .sort();

    const missing: SelectorUsage[] = [];
    for (const file of specFiles) {
      const fullPath = path.join(e2eTestsDir, file);
      const content = fs.readFileSync(fullPath, 'utf8');
      missing.push(...checkSelectors(content, file, webSrc));
    }

    const failureDetails = missing
      .map((m) => `  ${m.file}:${m.line} -> ${m.value}`)
      .join('\n');

    expect(
      missing,
      `Expected all e2e selectors to exist in web/src, but found missing:\n${failureDetails}`,
    ).toEqual([]);
  });

  it('fails on a fixture spec containing an unknown selector', () => {
    const webSrc = loadCombinedWebSrc();
    const fixtureSpec = [
      '// fixture spec',
      'const el = page.locator(\'[data-testid="no-such-id-5712"]\');',
      'await el.click();',
    ].join('\n');

    const missing = checkSelectors(fixtureSpec, 'fixture.spec.ts', webSrc);
    expect(missing).toEqual([
      {
        file: 'fixture.spec.ts',
        line: 2,
        value: 'no-such-id-5712',
      },
    ]);
  });
});
