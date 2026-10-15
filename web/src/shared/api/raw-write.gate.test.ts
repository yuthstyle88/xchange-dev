import { describe, expect, it, vi } from 'vitest';

export const STALE_KNOWN_FAILS = false;

// RW-1 (docs/design/xchange-v3-web.md §12.23): frozen at the 5 known raw-write files on main.
// Sorted alphabetically, one path per line.
export const KNOWN: readonly string[] = [
  'web/src/screens/customer/book/Book.tsx',
  'web/src/screens/customer/home/HomeScreen.tsx',
  'web/src/screens/customer/sign-in/SignIn.tsx',
  'web/src/screens/customer/sign-in/SignUp.tsx',
  'web/src/screens/operations/verification/VerificationScreen.tsx',
];

export function normalizePath(p: string): string {
  const norm = p.replace(/\\/g, '/');
  if (norm.startsWith('web/')) {
    return norm;
  }
  if (norm.startsWith('/src/')) {
    return 'web' + norm;
  }
  if (norm.startsWith('src/')) {
    return 'web/' + norm;
  }
  return 'web/src/' + norm.replace(/^\//, '');
}

export function shouldScanFile(filePath: string): boolean {
  const norm = normalizePath(filePath);
  if (!norm.endsWith('.ts') && !norm.endsWith('.tsx')) {
    return false;
  }
  if (norm.endsWith('.test.ts') || norm.endsWith('.test.tsx')) {
    return false;
  }
  if (norm === 'web/src/shared/api/client.ts') {
    return false;
  }
  return true;
}

const RAW_WRITE_METHOD_RE = /\bmethod\s*:\s*['"](?:POST|PUT|PATCH|DELETE)['"]/;

export function isOffending(content: string): boolean {
  return RAW_WRITE_METHOD_RE.test(content);
}

export interface GateCheckResult {
  offenders: string[];
  newOffenders: string[];
  staleKnown: string[];
  passed: boolean;
  errors: string[];
}

export function checkGate(
  files: Record<string, string>,
  known: readonly string[],
  staleKnownFails: boolean,
): GateCheckResult {
  const offenders: string[] = [];
  for (const [rawPath, content] of Object.entries(files)) {
    const filePath = normalizePath(rawPath);
    if (!shouldScanFile(filePath)) {
      continue;
    }
    if (isOffending(content)) {
      offenders.push(filePath);
    }
  }
  offenders.sort();

  const normalizedKnown = known.map(normalizePath);
  const sortedKnown = [...normalizedKnown].sort();
  const isKnownSorted = normalizedKnown.every((item, idx) => item === sortedKnown[idx]);

  const knownSet = new Set(normalizedKnown);
  const offendersSet = new Set(offenders);

  const newOffenders = offenders.filter((f) => !knownSet.has(f));
  const staleKnown = normalizedKnown.filter((f) => !offendersSet.has(f));

  const errors: string[] = [];
  if (!isKnownSorted) {
    errors.push('KNOWN list must be sorted alphabetically');
  }
  if (newOffenders.length > 0) {
    errors.push(`New offending files found not in KNOWN:\n  ${newOffenders.join('\n  ')}`);
  }
  if (staleKnownFails && staleKnown.length > 0) {
    errors.push(
      `Clean files still in KNOWN with STALE_KNOWN_FAILS=true:\n  ${staleKnown.join('\n  ')}`,
    );
  }
  if (!staleKnownFails && staleKnown.length > 0) {
    console.warn('Files in KNOWN that no longer offend:\n  ' + staleKnown.join('\n  '));
  }

  return {
    offenders,
    newOffenders,
    staleKnown,
    passed: errors.length === 0,
    errors,
  };
}

describe('RW-1 raw-write gate (real source tree)', () => {
  it('no new offending files outside of KNOWN exist in web/src', () => {
    const realFiles = import.meta.glob<string>('/src/**/*.{ts,tsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const result = checkGate(realFiles, KNOWN, STALE_KNOWN_FAILS);

    expect(KNOWN).toHaveLength(5);
    expect([...KNOWN].sort()).toEqual(KNOWN);
    expect(result.newOffenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});

describe('RW-1 raw-write gate (§9 behaviour table)', () => {
  it("row 1: a new file with fetch('/x', {method:'POST'}) fails the gate with 'New offending files'", () => {
    expect(isOffending("fetch('/x', {method:'POST'})")).toBe(true);
    expect(isOffending("fetch('/x', { method: 'PUT' })")).toBe(true);
    expect(isOffending("fetch('/x', { method: 'PATCH' })")).toBe(true);

    const fixtures: Record<string, string> = {
      'web/src/screens/new/NewScreen.tsx': "await fetch('/x', {method:'POST'});",
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(false);
    expect(result.newOffenders).toEqual(['web/src/screens/new/NewScreen.tsx']);
    expect(result.errors.join('\n')).toContain('New offending files');
  });

  it("row 2: the same through an alias, doFetch(url, { method: 'DELETE' }), fails", () => {
    expect(isOffending("doFetch(url, { method: 'DELETE' })")).toBe(true);
    expect(isOffending("fetchFn('/api/app/register', { method: 'POST' })")).toBe(true);
    expect(isOffending("function postJson() { return f(u, { method: 'POST' }); }")).toBe(true);

    const fixtures: Record<string, string> = {
      'web/src/screens/new/AliasScreen.tsx': "await doFetch(url, { method: 'DELETE' });",
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(false);
    expect(result.newOffenders).toEqual(['web/src/screens/new/AliasScreen.tsx']);
    expect(result.errors.join('\n')).toContain('New offending files');
  });

  it("row 3: fetch('/x') (GET) or fetch('/x', {method:'GET'}) passes", () => {
    expect(isOffending("fetch('/x')")).toBe(false);
    expect(isOffending("fetch('/x', {method:'GET'})")).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/screens/read/ReadScreen.tsx':
        "await fetch('/x'); await fetch('/x', {method:'GET'});",
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
    expect(result.newOffenders).toEqual([]);
  });

  it('row 4: web/src/shared/api/client.ts with a POST passes (excluded)', () => {
    expect(shouldScanFile('web/src/shared/api/client.ts')).toBe(false);
    expect(shouldScanFile('/src/shared/api/client.ts')).toBe(false);
    expect(shouldScanFile('src/shared/api/client.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/shared/api/client.ts': "await fetch('/api/v3/commands/x', { method: 'POST' });",
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('row 5: a *.test.tsx with a POST passes (tests excluded)', () => {
    expect(shouldScanFile('web/src/screens/customer/book/Book.test.tsx')).toBe(false);
    expect(shouldScanFile('web/src/shared/api/client.test.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/screens/customer/book/Book.test.tsx': "await fetch('/x', { method: 'POST' });",
      'web/src/shared/api/client.test.ts': "await fetch('/x', { method: 'DELETE' });",
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('row 6: a file in KNOWN that is now clean, flag false, warns and passes', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/book/Book.tsx': 'await api.post(url, body);',
    };
    const known = ['web/src/screens/customer/book/Book.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(true);
    expect(result.staleKnown).toEqual(['web/src/screens/customer/book/Book.tsx']);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it("row 7: the same with flag true fails with 'Clean files still in KNOWN'", () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/book/Book.tsx': 'await api.post(url, body);',
    };
    const known = ['web/src/screens/customer/book/Book.tsx'];
    const result = checkGate(fixtures, known, true);

    expect(result.passed).toBe(false);
    expect(result.staleKnown).toEqual(['web/src/screens/customer/book/Book.tsx']);
    expect(result.errors.join('\n')).toContain('Clean files still in KNOWN');
  });

  it('row 8: KNOWN unsorted fails', () => {
    expect([...KNOWN].sort()).toEqual(KNOWN);

    const unsortedKnown = [
      'web/src/screens/customer/home/HomeScreen.tsx',
      'web/src/screens/customer/book/Book.tsx',
    ];
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/book/Book.tsx': "fetch('/x', { method: 'POST' });",
      'web/src/screens/customer/home/HomeScreen.tsx': "fetch('/x', { method: 'POST' });",
    };
    const result = checkGate(fixtures, unsortedKnown, false);

    expect(result.passed).toBe(false);
    expect(result.errors.join('\n')).toContain('KNOWN list must be sorted');
  });
});
