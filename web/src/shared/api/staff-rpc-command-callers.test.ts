// Guard test (#4333, #4340): a web screen must not call a v3 command whose entry path is staff-only
// (`EntryPath::Staff*`), unless the screen itself is staff-only and allow-listed below.
// A `StaffRpc` command reads `callers.staff_user` (`api/src/presentation/http/v3/caller.rs:42-45`),
// which is `None` for the owner and answers `401 employeeAuthRequired`.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  existsSync(file: string): boolean;
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
let COMMANDS_DIR: string;

/**
 * Allow-list: repo-relative web files permitted to call `EntryPath::Staff*` v3 commands,
 * mapped to the reason (must be a staff-only screen).
 */
export const ALLOWED_STAFF_COMMAND_CALLERS: Record<string, string> = {};

export interface StaffCommandRoute {
  file: string;
  line: number;
  route: string;
  path: string;
}

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const currentDir = path.dirname(new URL(import.meta.url).pathname);
  SRC_DIR = path.resolve(currentDir, '../..');
  REPO_DIR = path.resolve(currentDir, '../../../..');
  COMMANDS_DIR = path.join(REPO_DIR, 'api/src/presentation/http/v3/commands');
});

function findRustFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...findRustFiles(fullPath));
    } else if (entry.name.endsWith('.rs')) {
      files.push(fullPath);
    }
  }
  return files.sort();
}

function findSourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const normalized = fullPath.replace(/\\/g, '/');
      if (normalized.includes('shared/api/generated') || normalized.includes('shared/testing/fixtures')) {
        continue;
      }
      files.push(...findSourceFiles(fullPath));
    } else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.includes('.test.')) {
      files.push(fullPath);
    }
  }
  return files.sort();
}

export function staffOnlyCommandRoutes(): {
  blockCount: number;
  nonLiteralRoutes: string[];
  routes: StaffCommandRoute[];
} {
  const rsFiles = findRustFiles(COMMANDS_DIR);
  let blockCount = 0;
  const nonLiteralRoutes: string[] = [];
  const routes: StaffCommandRoute[] = [];

  for (const fullPath of rsFiles) {
    const relFile = path.relative(REPO_DIR, fullPath).replace(/\\/g, '/');
    const content = fs.readFileSync(fullPath, 'utf8');
    const parts = content.split('impl CommandAction for');

    for (let i = 1; i < parts.length; i++) {
      blockCount++;
      const block = parts[i]!;
      const inheritsMatch = block.match(
        /const\s+INHERITS\s*:\s*\(\s*EntryPath\s*,\s*&'static\s+str\s*\)\s*=\s*\(\s*EntryPath::(\w+)/,
      );
      if (!inheritsMatch) continue;
      const entryPath = inheritsMatch[1]!;
      if (!entryPath.startsWith('Staff')) continue;

      const prefix =
        parts.slice(0, i).join('impl CommandAction for') +
        'impl CommandAction for' +
        block.slice(0, inheritsMatch.index ?? 0);
      const line = prefix.split('\n').length;

      const routeMatch = block.match(/const\s+ROUTE\s*:\s*&'static\s+str\s*=\s*"([^"]+)"\s*;/);
      if (!routeMatch) {
        nonLiteralRoutes.push(
          `${relFile}:${line}: EntryPath::${entryPath} has a non-literal ROUTE; make ROUTE a literal so the guard can read it`,
        );
        continue;
      }

      routes.push({
        file: relFile,
        line,
        route: routeMatch[1]!,
        path: entryPath,
      });
    }
  }

  return { blockCount, nonLiteralRoutes, routes };
}

export function routeRegex(route: string): RegExp {
  const pattern = route
    .split(/\{[^}]+\}/)
    .map((segment) => segment.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('(?:\\$\\{[^}]+\\}|[^/\'"`\\s]+)');
  return new RegExp(`${pattern}(?=['"\`?])`);
}

describe('staff-only v3 command caller guard (#4333, #4340)', () => {
  it('web screens outside the allow-list do not call EntryPath::Staff* v3 commands', () => {
    const { blockCount, nonLiteralRoutes, routes } = staffOnlyCommandRoutes();
    expect(blockCount, `parsed ${blockCount} CommandAction blocks under ${COMMANDS_DIR}`).toBeGreaterThanOrEqual(100);
    expect(nonLiteralRoutes, nonLiteralRoutes.join('\n')).toEqual([]);

    const files = findSourceFiles(SRC_DIR);
    expect(files.length, `scanned ${SRC_DIR}`).toBeGreaterThan(100);

    const compiled = routes.map((cmd) => ({ cmd, re: routeRegex(cmd.route) }));
    const violations: string[] = [];

    for (const file of files) {
      const relPath = path.relative(REPO_DIR, file).replace(/\\/g, '/');
      const webRelPath = relPath.replace(/^web\//, '');
      if (relPath in ALLOWED_STAFF_COMMAND_CALLERS || webRelPath in ALLOWED_STAFF_COMMAND_CALLERS) {
        continue;
      }

      const content = fs.readFileSync(file, 'utf8');
      for (const { cmd, re } of compiled) {
        if (re.test(content)) {
          violations.push(
            `${relPath}: calls ${cmd.route} (${cmd.file}:${cmd.line}, EntryPath::${cmd.path}) — make the command inherit EntryPath::Rpc (admits the owner and staff alike, caller.rs:42-45), or, if the screen is staff-only, add the file to ALLOWED_STAFF_COMMAND_CALLERS with the reason.`,
          );
        }
      }
    }

    expect(violations, violations.join('\n')).toEqual([]);
  });

  it('every allow-listed file still exists and still calls a staff-only command route (no stale entries)', () => {
    const { routes } = staffOnlyCommandRoutes();
    const matchers = routes.map((r) => routeRegex(r.route));

    for (const rel of Object.keys(ALLOWED_STAFF_COMMAND_CALLERS)) {
      const fullPath = path.join(REPO_DIR, rel);
      expect(fs.existsSync(fullPath), `${rel} no longer exists; drop it from the allow-list`).toBe(true);
      const content = fs.readFileSync(fullPath, 'utf8');
      const callsStaffRoute = matchers.some((re) => re.test(content));
      expect(callsStaffRoute, `${rel} no longer calls a staff-only command route; drop it from the allow-list`).toBe(
        true,
      );
    }
  });
});
