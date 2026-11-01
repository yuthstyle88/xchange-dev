import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
  readdirSync(dir: string): string[];
}

interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
  resolve(...segments: string[]): string;
}

declare const process: { versions: { node: string } };

let fs: NodeFs;
let path: NodePath;
let here: string;

async function getEnv() {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  const repoRoot = path.resolve(here, '../../..');
  return { fs, path, repoRoot };
}

export function extractNodeVersions(content: string): string[] {
  const matches: string[] = [];
  const regex = /^\s*node-version:\s*['"]?([0-9][^'"\s#]*)/gm;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(content)) !== null) {
    if (m[1]) {
      matches.push(m[1]);
    }
  }
  return matches;
}

export function extractMajor(version: string): string {
  return version.split('.')[0]!;
}

export function parseNvmrc(content: string | null | undefined, filename = '.nvmrc'): { major: string; full: string } {
  if (content == null || content.trim() === '') {
    throw new Error(`${filename} is missing or empty`);
  }
  const trimmed = content.trim();
  const match = trimmed.match(/^(\d+)\.\d+\.\d+$/);
  if (!match) {
    throw new Error(`${filename} must be formatted as MAJOR.MINOR.PATCH, got "${trimmed}"`);
  }
  return { major: match[1]!, full: trimmed };
}

export function checkWorkflowVersions(
  expectedMajor: string,
  workflows: Array<{ file: string; content: string }>
): { count: number } {
  let count = 0;
  for (const { file, content } of workflows) {
    const versions = extractNodeVersions(content);
    for (const v of versions) {
      count++;
      const major = extractMajor(v);
      if (major !== expectedMajor) {
        throw new Error(
          `Workflow ${file} specifies node-version "${v}" (major ${major}), expected major ${expectedMajor} from .nvmrc`
        );
      }
    }
  }
  if (count === 0) {
    throw new Error('No node-version declarations found across workflows');
  }
  return { count };
}

export function checkRunningNodeMajor(runningVersion: string, pinned: string): void {
  if (extractMajor(runningVersion) !== extractMajor(pinned)) {
    throw new Error(`Node ${runningVersion} is running; this repo pins ${pinned} (.nvmrc) — run \`nvm use\``);
  }
}

describe('node-pin', () => {
  it('P2 (drift row P3): every node-version in .github/workflows/*.yml matches .nvmrc major', async () => {
    const { fs, path, repoRoot } = await getEnv();
    const nvmrcPath = path.join(repoRoot, '.nvmrc');
    expect(fs.existsSync(nvmrcPath), '.nvmrc exists').toBe(true);
    const nvmrc = parseNvmrc(fs.readFileSync(nvmrcPath, 'utf8'));

    const workflowsDir = path.join(repoRoot, '.github', 'workflows');
    expect(fs.existsSync(workflowsDir), '.github/workflows exists').toBe(true);
    const ymlFiles = fs.readdirSync(workflowsDir).filter(f => f.endsWith('.yml') || f.endsWith('.yaml'));

    const workflows = ymlFiles.map(f => ({
      file: `.github/workflows/${f}`,
      content: fs.readFileSync(path.join(workflowsDir, f), 'utf8'),
    }));

    const result = checkWorkflowVersions(nvmrc.major, workflows);
    expect(result.count).toBeGreaterThan(0);
  });

  it('P3: one workflow says a different major version fails, naming file and values compared', () => {
    expect(extractNodeVersions("  node-version: '24'")).toEqual(['24']);
    expect(extractNodeVersions('  node-version: "24.x"')).toEqual(['24.x']);
    expect(extractNodeVersions('  node-version: 24 # comment')).toEqual(['24']);
    expect(extractNodeVersions('  node-version-file: .nvmrc')).toEqual([]);

    const mockWorkflows = [
      { file: '.github/workflows/web.yml', content: '  node-version: 25\n' },
    ];
    expect(() => checkWorkflowVersions('24', mockWorkflows)).toThrowError(
      /\.github\/workflows\/web\.yml.*"25".*major 25.*expected major 24/
    );
  });

  it('P4: .nvmrc missing or not MAJOR.MINOR.PATCH fails, naming .nvmrc', () => {
    expect(() => parseNvmrc('')).toThrowError(/\.nvmrc/);
    expect(() => parseNvmrc(null as unknown as string)).toThrowError(/\.nvmrc/);
    expect(() => parseNvmrc('24\n')).toThrowError(/\.nvmrc/);
    expect(() => parseNvmrc('v24.21.0\n')).toThrowError(/\.nvmrc/);
    expect(() => parseNvmrc('24.21\n')).toThrowError(/\.nvmrc/);
    expect(() => parseNvmrc('latest\n')).toThrowError(/\.nvmrc/);
  });

  it('P5: running Node major (process.versions.node) matches .nvmrc major', async () => {
    const { fs, path, repoRoot } = await getEnv();
    const nvmrcPath = path.join(repoRoot, '.nvmrc');
    expect(fs.existsSync(nvmrcPath), '.nvmrc exists').toBe(true);
    const nvmrc = parseNvmrc(fs.readFileSync(nvmrcPath, 'utf8'));
    checkRunningNodeMajor(process.versions.node, nvmrc.full);
  });

  it('P6: workflows contain no node-version: fails', () => {
    const mockWorkflows = [
      { file: '.github/workflows/noop.yml', content: 'name: Noop\non: push\n' },
      { file: '.github/workflows/file-ref.yml', content: '  node-version-file: .nvmrc\n' },
    ];
    expect(() => checkWorkflowVersions('24', mockWorkflows)).toThrowError(
      /No node-version declarations found/
    );
  });
});
