import { describe, expect, it } from 'vitest';

// PH-gate (docs/design/web-public-get.md §2 PH6, §5 PH-gate): nothing outside public-host.ts may read /api/public/host.
// KNOWN must stay empty: every caller was converted in PH-0..11.
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
  const norm = normalizePath(filePath);
  if (!norm.endsWith('.ts') && !norm.endsWith('.tsx')) {
    return false;
  }
  if (norm.endsWith('.test.ts') || norm.endsWith('.test.tsx')) {
    return false;
  }
  if (norm === 'web/src/shared/api/public-host.ts') {
    return false;
  }
  return true;
}

export function stripComments(source: string): string {
  let result = '';
  let i = 0;
  const len = source.length;
  let state: 'code' | 'single_quote' | 'double_quote' | 'template' | 'line_comment' | 'block_comment' = 'code';

  while (i < len) {
    const ch = source[i];
    const next = source[i + 1];

    if (state === 'code') {
      if (ch === '/' && next === '/') {
        state = 'line_comment';
        result += '  ';
        i += 2;
      } else if (ch === '/' && next === '*') {
        state = 'block_comment';
        result += '  ';
        i += 2;
      } else if (ch === "'") {
        state = 'single_quote';
        result += ch;
        i++;
      } else if (ch === '"') {
        state = 'double_quote';
        result += ch;
        i++;
      } else if (ch === '`') {
        state = 'template';
        result += ch;
        i++;
      } else {
        result += ch;
        i++;
      }
    } else if (state === 'line_comment') {
      if (ch === '\n' || ch === '\r') {
        state = 'code';
        result += ch;
      } else {
        result += ' ';
      }
      i++;
    } else if (state === 'block_comment') {
      if (ch === '*' && next === '/') {
        state = 'code';
        result += '  ';
        i += 2;
        continue;
      } else if (ch === '\n' || ch === '\r') {
        result += ch;
      } else {
        result += ' ';
      }
      i++;
    } else if (state === 'single_quote') {
      result += ch;
      if (ch === '\\') {
        result += next ?? '';
        i += 2;
        continue;
      }
      if (ch === "'") {
        state = 'code';
      }
      i++;
    } else if (state === 'double_quote') {
      result += ch;
      if (ch === '\\') {
        result += next ?? '';
        i += 2;
        continue;
      }
      if (ch === '"') {
        state = 'code';
      }
      i++;
    } else if (state === 'template') {
      result += ch;
      if (ch === '\\') {
        result += next ?? '';
        i += 2;
        continue;
      }
      if (ch === '`') {
        state = 'code';
      }
      i++;
    }
  }

  return result;
}

// An opening quote, then the path, then anything but more path: a query, `${...}` or the closing quote.
export const QUOTED_HOST_RE = /(['"`])\/api\/public\/host(?![\w/.-])/;

export interface OffendingLine {
  file: string;
  line: number;
  text: string;
}

export function findOffenses(filePath: string, content: string): OffendingLine[] {
  const norm = normalizePath(filePath);
  if (!shouldScanFile(norm)) {
    return [];
  }

  const cleaned = stripComments(content);
  const lines = cleaned.split('\n');
  const rawLines = content.split('\n');
  const offenses: OffendingLine[] = [];

  for (let idx = 0; idx < lines.length; idx++) {
    const line = lines[idx]!;
    if (QUOTED_HOST_RE.test(line)) {
      offenses.push({
        file: norm,
        line: idx + 1,
        text: rawLines[idx]?.trim() ?? '',
      });
    }
  }

  return offenses;
}

export interface GateCheckResult {
  offenders: string[];
  passed: boolean;
  errors: string[];
}

export function checkGate(
  files: Record<string, string>,
  known: readonly string[] = KNOWN,
): GateCheckResult {
  const offenders: string[] = [];
  const errors: string[] = [];

  if (known.length > 0) {
    errors.push(`KNOWN list must stay empty, but contained ${known.length} entries`);
  }

  for (const [rawPath, content] of Object.entries(files)) {
    const fileOffenses = findOffenses(rawPath, content);
    for (const offense of fileOffenses) {
      offenders.push(`${offense.file}:${offense.line}`);
    }
  }

  offenders.sort();

  if (offenders.length > 0) {
    errors.push(
      `Offending files reading /api/public/host directly:\n  ${offenders.join('\n  ')}\nUse readHostOrg from shared/api/public-host.ts instead.`,
    );
  }

  return {
    offenders,
    passed: errors.length === 0,
    errors,
  };
}

describe('PH-gate public-host gate (real source tree)', () => {
  it('no non-test file outside public-host.ts reads /api/public/host', () => {
    const realFiles = import.meta.glob<string>('/src/**/*.{ts,tsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const result = checkGate(realFiles, KNOWN);

    expect(KNOWN).toEqual([]);
    expect(result.offenders, result.errors.join('\n')).toEqual([]);
    expect(result.errors).toEqual([]);
    expect(result.passed).toBe(true);
  });
});

describe('PH-gate public-host gate unit tests', () => {
  it('single-quoted /api/public/host fails and names file:line with readHostOrg guidance', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/new/Screen.tsx': "const x = 1;\nawait fetch('/api/public/host');",
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(false);
    expect(result.offenders).toEqual(['web/src/screens/customer/new/Screen.tsx:2']);
    expect(result.errors[0]).toContain('web/src/screens/customer/new/Screen.tsx:2');
    expect(result.errors[0]).toContain('Use readHostOrg from shared/api/public-host.ts instead.');
  });

  it('double-quoted "/api/public/host" fails and names file:line', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/new/Screen.tsx': 'await fetch("/api/public/host");',
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(false);
    expect(result.offenders).toEqual(['web/src/screens/customer/new/Screen.tsx:1']);
  });

  it('backtick-quoted `/api/public/host` fails and names file:line', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/new/Screen.tsx': 'await fetch(`/api/public/host`);',
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(false);
    expect(result.offenders).toEqual(['web/src/screens/customer/new/Screen.tsx:1']);
  });

  it('a template with interpolation fails, and a longer path does not', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/new/Screen.tsx': 'fetch(`/api/public/host${q}`);\nfetch(\'/api/public/hosts\');',
    };
    expect(checkGate(fixtures).offenders).toEqual(['web/src/screens/customer/new/Screen.tsx:1']);
  });

  it('a block comment that ends a line keeps the line numbers', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/new/Screen.tsx': "/** a */\n/* b */\nconst x = 1;\nfetch('/api/public/host');",
    };
    expect(checkGate(fixtures).offenders).toEqual(['web/src/screens/customer/new/Screen.tsx:4']);
  });

  it('web/src/shared/api/public-host.ts with /api/public/host passes (exempt)', () => {
    expect(shouldScanFile('web/src/shared/api/public-host.ts')).toBe(false);
    expect(shouldScanFile('/src/shared/api/public-host.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/shared/api/public-host.ts': "await api.publicGet('/api/public/host');",
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('test files (*.test.ts, *.test.tsx) pass (exempt)', () => {
    expect(shouldScanFile('web/src/screens/customer/home/HomeScreen.test.tsx')).toBe(false);
    expect(shouldScanFile('web/src/shared/api/public-host.test.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/screens/customer/home/HomeScreen.test.tsx': "if (url === '/api/public/host') return json(200, { org: 'XC' });",
      'web/src/shared/api/public-host.test.ts': "expect(calls[0].url).toBe('/api/public/host');",
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('comments mentioning /api/public/host pass', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/customer/queue/queue-api.ts': [
        '// Line comment mentions "/api/public/host"',
        '/** JSDoc mentions (`/api/public/host`) */',
        '/* Block comment mentions \'/api/public/host\' */',
        'export function host() { return "ok"; }',
      ].join('\n'),
    };
    const result = checkGate(fixtures);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('non-empty KNOWN fails the gate', () => {
    const fixtures: Record<string, string> = {};
    const result = checkGate(fixtures, ['web/src/screens/customer/home/HomeScreen.tsx']);

    expect(result.passed).toBe(false);
    expect(result.errors.join('\n')).toContain('KNOWN list must stay empty');
  });
});
