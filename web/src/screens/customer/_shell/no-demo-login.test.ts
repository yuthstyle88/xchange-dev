// APP-7 (#998): no demo credentials in customer web/src or web/pages (design `customer-app.md` §17, §9.1 row APP-7).
// The built bundle must contain neither the demo OTP nor the demo sign-in slug.
// Written before the implementation by test-writer gemini/warut-6 on claude/5edf7bb5's order.
// This is a source scan over web/src and web/pages; it must PASS today as a guard.
// The forbidden tokens are built from pieces so this test file itself contains neither.
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
  extname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let srcDir: string;
let pagesDir: string;

// The forbidden tokens are constructed from pieces so this file itself does not contain them:
// 1. the demo OTP
// 2. the demo sign-in slug
const FORBIDDEN_TOKENS = [
  ['24', '68', '10'].join(''),
  ['demo', 'login'].join('-'),
];

const TEXT_EXTENSIONS = new Set([
  '.ts',
  '.tsx',
  '.js',
  '.jsx',
  '.html',
  '.css',
  '.json',
  '.md',
  '.svg',
]);

function walk(dir: string): string[] {
  let files: string[] = [];
  if (!fs.existsSync(dir)) return files;
  for (const entry of fs.readdirSync(dir)) {
    const full = path.join(dir, entry);
    const stat = fs.statSync(full);
    if (stat.isDirectory()) {
      files = files.concat(walk(full));
    } else if (stat.isFile()) {
      const ext = path.extname(full).toLowerCase();
      if (TEXT_EXTENSIONS.has(ext)) {
        files.push(full);
      }
    }
  }
  return files;
}

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisFile = new URL(import.meta.url).pathname;
  const thisDir = path.dirname(thisFile);
  // thisDir: web/src/screens/customer/_shell
  srcDir = path.resolve(thisDir, '../../..');
  pagesDir = path.resolve(srcDir, '../pages');
});

describe('APP-7 guard: no demo credentials in web/src or web/pages', () => {
  it('G1 the scan reaches source files in both web/src and web/pages', () => {
    const srcFiles = walk(srcDir);
    const pagesFiles = walk(pagesDir);
    expect(srcFiles.length).toBeGreaterThan(50);
    expect(pagesFiles.length).toBeGreaterThan(0);
    const relativeSrc = srcFiles.map((f) => path.join('src', path.resolve(f).slice(srcDir.length)));
    expect(relativeSrc).toContain('src/screens/customer/rate-display/RateDisplay.tsx');
  });

  it('G2 no file under web/src or web/pages contains forbidden demo credentials', () => {
    const allFiles = [...walk(srcDir), ...walk(pagesDir)];
    const hits: string[] = [];

    for (const file of allFiles) {
      const content = fs.readFileSync(file, 'utf8');
      const lines = content.split('\n');
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        for (const token of FORBIDDEN_TOKENS) {
          if (line.includes(token)) {
            const rel = file.includes('/pages/')
              ? 'pages/' + path.resolve(file).slice(pagesDir.length + 1)
              : 'src/' + path.resolve(file).slice(srcDir.length + 1);
            hits.push(`${rel}:${i + 1} matches "${token}": ${line.trim()}`);
          }
        }
      }
    }

    expect(hits).toEqual([]);
  });

  it('G3 the detector catches forbidden tokens when present', () => {
    const sampleOtp = `const otp = '${['24', '68', '10'].join('')}';`;
    const sampleDemo = `const path = '/${['demo', 'login'].join('-')}';`;
    const hasForbidden = (s: string) => FORBIDDEN_TOKENS.some((t) => s.includes(t));
    expect(hasForbidden(sampleOtp)).toBe(true);
    expect(hasForbidden(sampleDemo)).toBe(true);
    expect(hasForbidden('const normal = true;')).toBe(false);
  });
});
