import { describe, expect, it, vi } from 'vitest';

export const STALE_KNOWN_FAILS = true;

// WE5: KNOWN list emptied in WE-final (#1881) once all 45 WE-n folders and WE-form merged.
// Sorted alphabetically, one path per line (design §4.1).
export const KNOWN: readonly string[] = [];

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
  const norm = filePath.replace(/\\/g, '/');
  if (!norm.endsWith('.ts') && !norm.endsWith('.tsx')) {
    return false;
  }
  if (norm.endsWith('.test.ts') || norm.endsWith('.test.tsx')) {
    return false;
  }
  if (norm.endsWith('worded-error.ts')) {
    return false;
  }
  // WE4 / documents §A.7 (#1784 issuecomment-6179288474): web/src/screens/documents/refusal.ts
  // defines refusalText for the documents screen's own refusal table (mapping 400 invalidRequest
  // without field to documents.badBranch and non-5xx server messages per §A.7 before falling
  // through to wordedError).
  if (norm.endsWith('screens/documents/refusal.ts')) {
    return false;
  }
  return true;
}

export function isOffending(filePath: string, content: string): boolean {
  // Signal 1: String(err), String(e) or String(error)
  if (/String\((?:err|e|error)\)/.test(content)) {
    return true;
  }
  // Signal 2: definition of toWordedError, toErrorText or refusalText
  if (
    /(?:function|const|let|var)\s+(?:toWordedError|toErrorText|refusalText)\b|\basync\s+function\s+(?:toWordedError|toErrorText|refusalText)\b|\b(?:toWordedError|toErrorText|refusalText)\s*[:=]/.test(
      content,
    )
  ) {
    return true;
  }
  // Signal 3: any errorText( call outside web/src/shared/i18n/
  const isInsideI18n = filePath.includes('shared/i18n/');
  if (!isInsideI18n && /errorText\(/.test(content)) {
    return true;
  }
  return false;
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
    if (isOffending(filePath, content)) {
      offenders.push(filePath);
    }
  }
  offenders.sort();

  const normalizedKnown = known.map(normalizePath);
  const knownSet = new Set(normalizedKnown);
  const offendersSet = new Set(offenders);

  const newOffenders = offenders.filter((f) => !knownSet.has(f));
  const staleKnown = normalizedKnown.filter((f) => !offendersSet.has(f));

  const errors: string[] = [];
  if (newOffenders.length > 0) {
    errors.push(`New offending files found not in KNOWN:\n  ${newOffenders.join('\n  ')}`);
  }
  if (staleKnownFails && staleKnown.length > 0) {
    errors.push(`Clean files still in KNOWN with STALE_KNOWN_FAILS=true:\n  ${staleKnown.join('\n  ')}`);
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

describe('WE5 worded-error gate (real source tree)', () => {
  it('KNOWN is sorted', () => {
    expect([...KNOWN].sort()).toEqual(KNOWN);
  });

  it('no new offending files outside of KNOWN exist in web/src', () => {
    const realFiles = import.meta.glob<string>('/src/**/*.{ts,tsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const result = checkGate(realFiles, KNOWN, STALE_KNOWN_FAILS);

    expect(result.newOffenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});

describe('WE5 gate unit tests (fixtures)', () => {
  it('a new offender fails', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = String(err);',
      'web/src/screens/new/NewScreen.tsx': 'const msg = String(e);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(false);
    expect(result.newOffenders).toEqual(['web/src/screens/new/NewScreen.tsx']);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('a clean KNOWN file passes while the flag is false', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = wordedError(err);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(true);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/AdminShell.tsx']);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('the same file fails when the flag is true', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = wordedError(err);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, true);

    expect(result.passed).toBe(false);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/AdminShell.tsx']);
    expect(result.errors.some((e) => e.includes('STALE_KNOWN_FAILS=true'))).toBe(true);
  });

  it('each of the three signals is detected', () => {
    // Signal 1: String(err), String(e), String(error)
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(err);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(e);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(error);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(other);')).toBe(false);

    // Signal 2: definition of toWordedError, toErrorText or refusalText
    expect(isOffending('web/src/screens/A.tsx', 'function toWordedError(err: unknown) { return ""; }')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const toErrorText = (e: unknown) => "";')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'export function refusalText(err: unknown) { return ""; }')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const helper = (err: unknown) => "";')).toBe(false);

    // Signal 3: any errorText( call outside web/src/shared/i18n/
    expect(isOffending('web/src/screens/A.tsx', 'return errorText(code, msg);')).toBe(true);
    expect(isOffending('web/src/shared/i18n/index.ts', 'return errorText(code, msg);')).toBe(false);
    expect(isOffending('web/src/shared/i18n/worded-error.ts', 'return errorText(code, msg);')).toBe(false);
  });

  it('form.ts and test files are not scanned', () => {
    // Form and test files should be excluded by shouldScanFile
    expect(shouldScanFile('web/src/screens/documents/refusal.ts')).toBe(false);
    expect(shouldScanFile('src/screens/documents/refusal.ts')).toBe(false);
    expect(shouldScanFile('/src/screens/documents/refusal.ts')).toBe(false);
    expect(shouldScanFile('web/src/screens/admin/AdminShell.test.tsx')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.test.ts')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.gate.test.ts')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.ts')).toBe(false);

    // Non-excluded source files are scanned
    expect(shouldScanFile('web/src/screens/admin/AdminShell.tsx')).toBe(true);

    // In checkGate, even if form.ts and test files contain signals, they are ignored
    const fixtures: Record<string, string> = {
      'web/src/screens/documents/refusal.ts': 'errorText(code, msg); String(err);',
      'web/src/screens/admin/AdminShell.test.tsx': 'String(err); toWordedError();',
      'web/src/shared/i18n/worded-error.ts': 'errorText(code, msg);',
    };
    const result = checkGate(fixtures, [], false);
    expect(result.offenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});
