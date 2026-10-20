// Guard test (#3970): only staff-only screens may read the staff_me view.
// Reading staff_me answers 401 employeeAuthRequired for the owner (EXECUTIVE/ADMIN), which causes
// permission checks to fail silently. Any screen the owner can open must read the caller from the
// me view instead (works for both staff and the owner).
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readdirSync(dir: string, opts: { withFileTypes: true }): { name: string; isDirectory(): boolean }[];
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  join(...segments: string[]): string;
  dirname(p: string): string;
  resolve(...segments: string[]): string;
  relative(from: string, to: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let SRC_DIR: string;
let REPO_DIR: string;

/**
 * Allow-list: screens permitted to read `staff_me`, with one line per file and its reason.
 * Add a file here only if the screen is staff-only.
 */
const ALLOWED_STAFF_ME_CALLERS: Record<string, string> = {
  'web/src/screens/desk/queue/QueueScreen.tsx': "the counter needs the teller's userId; staff only",
  'web/src/screens/staff/StaffPage.tsx': '/staff, the staff hub',
};

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const currentDir = path.dirname(new URL(import.meta.url).pathname);
  SRC_DIR = path.resolve(currentDir, '../..');
  REPO_DIR = path.resolve(currentDir, '../../../..');
});

function findSourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const normalized = fullPath.replace(/\\/g, '/');
      if (normalized.includes('shared/api/generated')) continue;
      files.push(...findSourceFiles(fullPath));
    } else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.includes('.test.')) {
      files.push(fullPath);
    }
  }
  return files;
}

describe('staff_me caller guard (#3970)', () => {
  it('only staff-only screens may read staff_me (prevents owner 401 bug class)', () => {
    const files = findSourceFiles(SRC_DIR);
    const violations: string[] = [];

    for (const file of files) {
      const relPath = path.relative(REPO_DIR, file).replace(/\\/g, '/');
      const webRelPath = relPath.replace(/^web\//, '');
      const isAllowed = relPath in ALLOWED_STAFF_ME_CALLERS || webRelPath in ALLOWED_STAFF_ME_CALLERS;

      if (isAllowed) continue;

      const content = fs.readFileSync(file, 'utf8');
      if (content.includes("'staff_me'") || content.includes('"staff_me"')) {
        violations.push(
          `${relPath}: read the caller from 'me' (works for staff and the owner); add a file here only if the screen is staff-only, with the reason.`,
        );
      }
    }

    expect(violations, violations.join('\n')).toEqual([]);
  });
});
