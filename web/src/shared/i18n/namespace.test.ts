// Static gate (#439): every `screenT('<ns>', table)` call under `web/src/screens/**`
// names the dotted path of its own folder (design `docs/design/xchange-v3-web.md` W10 point 1).
//
// `screenT` resolves through the collected lookup and falls back to the table it is handed
// (controller decision on #427), so a namespace that does not match its folder still renders
// and is never checked at run time. This test checks it where it is written.
//
// Rules:
// - The expected namespace of a file is the nearest folder, the file's own or an ancestor under
//   `src/screens/`, that holds an `i18n.ts`, with `/` turned into `.`. So
//   `screens/admin/payments/Payments.tsx` expects `admin.payments`, and a helper in
//   `screens/admin/payments/components/Row.tsx` (no `i18n.ts` in `components/`) expects
//   `admin.payments` too. A file that calls `screenT` with no `i18n.ts` in its folder or above
//   is a problem: the table it hands `screenT` cannot be its folder's.
// - The first argument must be a plain string literal ('…', "…", or a template with no `${}`).
//   Anything else cannot be checked and is a problem.
// - `screenT` may be imported, and called; an alias (`screenT as x`) or any other use as a value
//   is a problem, because calls through it could not be checked.
// - `screenT(` inside a comment or a string is not a call and is ignored.
// - Test files under `screens/` are held to the same rule.
import { describe, expect, it } from 'vitest';

const SCREENS_PREFIX = '/src/screens/';

// After one of these keywords a `/` starts a regex literal, not a division.
const REGEX_AFTER_KEYWORDS = new Set([
  'return', 'typeof', 'case', 'in', 'of', 'delete', 'void', 'throw', 'yield', 'await',
  'instanceof', 'new', 'else', 'do',
]);
const REGEX_AFTER_PUNCTUATION = '(,=:[!&|?{};+-*%<>~^';

function regexMayStartAfter(lastToken: string): boolean {
  if (lastToken === '') return true;
  if (lastToken.length === 1 && REGEX_AFTER_PUNCTUATION.includes(lastToken)) return true;
  return REGEX_AFTER_KEYWORDS.has(lastToken);
}

/** Blanks comments and the inside of string, template and regex literals, keeping offsets.
 *  Quote characters stay, so a literal's bounds are still visible; code inside `${}` stays. */
function maskSource(src: string): string {
  const out = src.split('');
  const blank = (i: number) => {
    if (out[i] !== '\n') out[i] = ' ';
  };
  // Stack of brace depths for `${` … `}` inside templates.
  const templateStack: number[] = [];
  let braceDepth = 0;
  // The last token seen in code: one punctuation character, or a whole word.
  let lastSignificant = '';
  let i = 0;
  const n = src.length;

  const scanTemplate = (start: number): number => {
    // `start` is just after the opening backtick or after a closing `}` of `${}`.
    let j = start;
    while (j < n) {
      const c = src[j];
      if (c === '\\') {
        blank(j);
        if (j + 1 < n) blank(j + 1);
        j += 2;
        continue;
      }
      if (c === '`') return j + 1;
      if (c === '$' && src[j + 1] === '{') {
        blank(j);
        blank(j + 1);
        templateStack.push(braceDepth);
        braceDepth += 1;
        return -(j + 2); // negative: resumed code at j+2
      }
      blank(j);
      j += 1;
    }
    return n;
  };

  while (i < n) {
    const c = src[i]!;
    const next = src[i + 1];
    if (c === '/' && next === '/') {
      while (i < n && src[i] !== '\n') blank(i++);
      continue;
    }
    if (c === '/' && next === '*') {
      const end = src.indexOf('*/', i + 2);
      const stop = end === -1 ? n : end + 2;
      while (i < stop) blank(i++);
      continue;
    }
    if (c === "'" || c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== c && src[j] !== '\n') {
        if (src[j] === '\\') {
          blank(j);
          j += 1;
        }
        if (j < n) blank(j);
        j += 1;
      }
      i = j + 1;
      lastSignificant = c;
      continue;
    }
    if (c === '`') {
      const r = scanTemplate(i + 1);
      if (r < 0) {
        i = -r;
        lastSignificant = '{';
      } else {
        i = r;
        lastSignificant = '`';
      }
      continue;
    }
    if (c === '/' && regexMayStartAfter(lastSignificant)) {
      // Regex literal.
      let j = i + 1;
      let inClass = false;
      while (j < n && src[j] !== '\n') {
        const d = src[j];
        if (d === '\\') {
          blank(j);
          if (j + 1 < n) blank(j + 1);
          j += 2;
          continue;
        }
        if (d === '[') inClass = true;
        else if (d === ']') inClass = false;
        else if (d === '/' && !inClass) break;
        blank(j);
        j += 1;
      }
      i = j + 1;
      lastSignificant = '/';
      continue;
    }
    if (c === '{') braceDepth += 1;
    if (c === '}') {
      braceDepth -= 1;
      if (templateStack.length > 0 && templateStack[templateStack.length - 1] === braceDepth) {
        templateStack.pop();
        blank(i);
        const r = scanTemplate(i + 1);
        if (r < 0) {
          i = -r;
          lastSignificant = '{';
        } else {
          i = r;
          lastSignificant = '`';
        }
        continue;
      }
    }
    if (/[\w$]/.test(c)) {
      let j = i;
      while (j < n && /[\w$]/.test(src[j]!)) j += 1;
      lastSignificant = src.slice(i, j);
      i = j;
      continue;
    }
    if (!/\s/.test(c)) lastSignificant = c;
    i += 1;
  }
  return out.join('');
}

function nextNonSpace(masked: string, from: number): number {
  let i = from;
  while (i < masked.length && /\s/.test(masked[i]!)) i += 1;
  return i;
}

/** Reads a plain string literal starting at `at` in the original source.
 *  Returns its value and the index after it, or null if it is not a plain literal. */
function readPlainLiteral(src: string, at: number): { value: string; end: number } | null {
  const q = src[at];
  if (q !== "'" && q !== '"' && q !== '`') return null;
  const close = src.indexOf(q, at + 1);
  if (close === -1) return null;
  const value = src.slice(at + 1, close);
  if (value.includes('\\') || value.includes('\n')) return null;
  if (q === '`' && value.includes('${')) return null;
  return { value, end: close + 1 };
}

/** Ranges of `import { … }` / `export { … }` clauses in masked source. */
function specifierRanges(masked: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  const re = /\b(?:import|export)\s+(?:type\s+)?\{[^}]*\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(masked)) !== null) ranges.push([m.index, m.index + m[0].length]);
  return ranges;
}

function expectedNamespace(path: string, files: Record<string, string>): string | null {
  const rel = path.slice(SCREENS_PREFIX.length);
  const dirs = rel.split('/').slice(0, -1);
  for (let k = dirs.length; k >= 1; k -= 1) {
    const folder = dirs.slice(0, k);
    if (Object.prototype.hasOwnProperty.call(files, `${SCREENS_PREFIX}${folder.join('/')}/i18n.ts`)) {
      return folder.join('.');
    }
  }
  return null;
}

/** Returns one problem per `screenT` call or use that does not name its own folder.
 *  `files` maps paths as `import.meta.glob` gives them (`/src/screens/<path>/<file>`) to raw source. */
function checkScreenNamespaces(files: Record<string, string>): string[] {
  const problems: string[] = [];
  for (const path of Object.keys(files).sort()) {
    const src = files[path]!;
    if (!path.startsWith(SCREENS_PREFIX)) {
      problems.push(`${path}: not under ${SCREENS_PREFIX}`);
      continue;
    }
    const masked = maskSource(src);
    const ranges = specifierRanges(masked);
    const re = /(?<![\w$])screenT(?![\w$])/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(masked)) !== null) {
      const at = m.index;
      const line = src.slice(0, at).split('\n').length;
      const where = `${path}:${line}`;
      const inSpecifier = ranges.some(([a, b]) => at >= a && at < b);
      let p = nextNonSpace(masked, at + 'screenT'.length);
      // `typeof screenT` (a type such as `ReturnType<typeof screenT<typeof i18n>>`) calls nothing
      // and cannot carry a namespace, so it is not a use to check.
      if (/(?<![\w$])typeof\s+$/.test(masked.slice(Math.max(0, at - 64), at))) continue;
      if (inSpecifier) {
        if (/^as(?![\w$])/.test(masked.slice(p))) {
          problems.push(`${where}: screenT is imported under an alias; calls through it cannot be checked`);
        }
        continue;
      }
      if (masked[p] === '<') {
        let depth = 0;
        for (; p < masked.length; p += 1) {
          if (masked[p] === '<') depth += 1;
          else if (masked[p] === '>') {
            depth -= 1;
            if (depth === 0) break;
          }
        }
        p = nextNonSpace(masked, p + 1);
      }
      if (masked[p] !== '(') {
        problems.push(`${where}: screenT is used as a value, not called; its namespace cannot be checked`);
        continue;
      }
      const argAt = nextNonSpace(masked, p + 1);
      const lit = readPlainLiteral(src, argAt);
      const after = lit ? masked[nextNonSpace(masked, lit.end)] : undefined;
      if (!lit || (after !== ',' && after !== ')')) {
        problems.push(`${where}: screenT's first argument is not a plain string literal; its namespace cannot be checked`);
        continue;
      }
      const expected = expectedNamespace(path, files);
      if (expected === null) {
        problems.push(
          `${where}: screenT namespace '${lit.value}' — no i18n.ts in this folder or any folder above it under src/screens/`,
        );
        continue;
      }
      if (lit.value !== expected) {
        problems.push(
          `${where}: screenT namespace '${lit.value}' — expected '${expected}' (nearest folder with i18n.ts)`,
        );
      }
    }
  }
  return problems;
}

const I18N = "export const th = { title: 'x' } as const;\nexport const en = { title: 'x' };\n";

describe('checkScreenNamespaces: the checker, on in-memory trees', () => {
  it('passes a screen whose namespace is its folder', () => {
    expect(
      checkScreenNamespaces({
        '/src/screens/login/i18n.ts': I18N,
        '/src/screens/login/Login.tsx':
          "import { screenT } from '../../shared/i18n';\nimport * as loginI18n from './i18n';\nconst t = screenT('login', loginI18n);\n",
      }),
    ).toEqual([]);
  });

  it('fails a mismatching namespace, naming the file, the namespace found and the one expected', () => {
    const problems = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': "const t = screenT('signin', loginI18n);\n",
    });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain('/src/screens/login/Login.tsx:1');
    expect(problems[0]).toContain("'signin'");
    expect(problems[0]).toContain("expected 'login'");
  });

  it('expects the full dotted path of a nested screen folder, not a part of it', () => {
    const files = {
      '/src/screens/admin/i18n.ts': I18N,
      '/src/screens/admin/payments/i18n.ts': I18N,
      '/src/screens/admin/payments/Payments.tsx': "const t = screenT('admin.payments', paymentsI18n);\n",
    };
    expect(checkScreenNamespaces(files)).toEqual([]);
    for (const wrong of ['admin', 'payments', 'admin/payments']) {
      const problems = checkScreenNamespaces({
        ...files,
        '/src/screens/admin/payments/Payments.tsx': `const t = screenT('${wrong}', paymentsI18n);\n`,
      });
      expect(problems).toHaveLength(1);
      expect(problems[0]).toContain("expected 'admin.payments'");
    }
  });

  it('a sub-folder without its own i18n.ts expects the nearest ancestor that has one', () => {
    const files = {
      '/src/screens/admin/i18n.ts': I18N,
      '/src/screens/admin/payments/i18n.ts': I18N,
      '/src/screens/admin/payments/components/Row.tsx': "const t = screenT('admin.payments', paymentsI18n);\n",
      '/src/screens/admin/shared/Bar.tsx': "const t = screenT('admin', adminI18n);\n",
    };
    expect(checkScreenNamespaces(files)).toEqual([]);
    const problems = checkScreenNamespaces({
      ...files,
      '/src/screens/admin/payments/components/Row.tsx':
        "const t = screenT('admin.payments.components', paymentsI18n);\n",
    });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("expected 'admin.payments'");
  });

  it('fails a first argument that is not a plain string literal', () => {
    const calls = [
      'screenT(ns, loginI18n)',
      "screenT('log' + 'in', loginI18n)",
      'screenT(`${ns}`, loginI18n)',
      "screenT(\n  // the namespace\n  NS.login, loginI18n)",
    ];
    for (const call of calls) {
      const problems = checkScreenNamespaces({
        '/src/screens/login/i18n.ts': I18N,
        '/src/screens/login/Login.tsx': `const t = ${call};\n`,
      });
      expect(problems, call).toHaveLength(1);
      expect(problems[0], call).toContain('not a plain string literal');
    }
  });

  it('accepts a type argument, double quotes, a plain template and comments around the literal', () => {
    const calls = [
      "screenT<typeof loginI18n>('login', loginI18n)",
      'screenT("login", loginI18n)',
      'screenT(`login`, loginI18n)',
      "screenT(/* ns */ 'login' /* table */, loginI18n)",
    ];
    for (const call of calls) {
      expect(
        checkScreenNamespaces({
          '/src/screens/login/i18n.ts': I18N,
          '/src/screens/login/Login.tsx': `const t = ${call};\n`,
        }),
        call,
      ).toEqual([]);
    }
  });

  it('fails a call with no i18n.ts in its folder or above', () => {
    for (const path of ['/src/screens/login/Login.tsx', '/src/screens/Loose.tsx']) {
      const problems = checkScreenNamespaces({ [path]: "const t = screenT('login', loginI18n);\n" });
      expect(problems, path).toHaveLength(1);
      expect(problems[0], path).toContain('no i18n.ts');
    }
  });

  it('ignores screenT( inside comments, strings, templates and regexes', () => {
    expect(
      checkScreenNamespaces({
        '/src/screens/login/i18n.ts': I18N,
        '/src/screens/login/Login.tsx': [
          "// const t = screenT('wrong', x);",
          "/* screenT('wrong', x) */",
          "const a = \"screenT('wrong', x)\";",
          "const b = 'screenT(\\'wrong\\', x)';",
          "const c = `screenT('wrong', x)`;",
          "const d = /screenT\\('wrong'/;",
          "const t = screenT('login', loginI18n);",
          '',
        ].join('\n'),
      }),
    ).toEqual([]);
  });

  it('still sees a call inside a template substitution', () => {
    const problems = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': "const s = `${screenT('wrong', loginI18n)('title')}`;\n",
    });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("expected 'login'");
  });

  it('fails an aliased import and a use as a value', () => {
    const aliased = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': "import { screenT as st } from '../../shared/i18n';\nconst t = st('wrong', x);\n",
    });
    expect(aliased).toHaveLength(1);
    expect(aliased[0]).toContain('alias');

    const asValue = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': "const make = screenT;\nconst t = make('wrong', x);\n",
    });
    expect(asValue).toHaveLength(1);
    expect(asValue[0]).toContain('used as a value');
  });

  it('accepts typeof screenT in a type alias and in a prop type annotation', () => {
    const files = {
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/components/Row.tsx': [
        "import { screenT } from '../../../shared/i18n';",
        "import type * as i18n from '../i18n';",
        'type T = ReturnType<typeof screenT<typeof i18n>>;',
        'type F = typeof screenT;',
        'export function Row(props: { t: ReturnType<typeof screenT<typeof i18n>>; label: string }) {',
        "  return props.t('title');",
        '}',
        'export type { T, F };',
        '',
      ].join('\n'),
    };
    expect(checkScreenNamespaces(files)).toEqual([]);
  });

  it('still fails a runtime use as a value next to a type use', () => {
    const problems = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': [
        'type T = ReturnType<typeof screenT<typeof i18n>>;',
        'register(screenT);',
        '',
      ].join('\n'),
    });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain('/src/screens/login/Login.tsx:2');
    expect(problems[0]).toContain('used as a value');
  });

  it('masks a regex literal after a keyword', () => {
    const keywords = ['return', 'typeof', 'case', 'in', 'of', 'delete', 'void', 'throw', 'yield', 'await'];
    for (const kw of keywords) {
      const problems = checkScreenNamespaces({
        '/src/screens/login/i18n.ts': I18N,
        '/src/screens/login/Login.tsx': `const t = screenT('login', loginI18n);\nx = ${kw} /screenT\\('wrong'/;\n`,
      });
      expect(problems, kw).toEqual([]);
    }
    expect(
      checkScreenNamespaces({
        '/src/screens/login/i18n.ts': I18N,
        '/src/screens/login/Login.tsx':
          "function f(s: string) {\n  return /screenT\\('x'/.test(s);\n}\nconst t = screenT('login', loginI18n);\n",
      }),
    ).toEqual([]);
  });

  it('still reads a division after a word as code', () => {
    const problems = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx': "const half = total / 2; const t = screenT('wrong', x); const q = n / 2;\n",
    });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("'wrong'");
  });

  it('checks every call in a file and reports each bad one with its line', () => {
    const problems = checkScreenNamespaces({
      '/src/screens/login/i18n.ts': I18N,
      '/src/screens/login/Login.tsx':
        "const a = screenT('login', x);\nconst b = screenT('nope', x);\nconst c = screenT('also', x);\n",
    });
    expect(problems).toEqual([
      "/src/screens/login/Login.tsx:2: screenT namespace 'nope' — expected 'login' (nearest folder with i18n.ts)",
      "/src/screens/login/Login.tsx:3: screenT namespace 'also' — expected 'login' (nearest folder with i18n.ts)",
    ]);
  });
});

describe('every screenT call under web/src/screens names its own folder', () => {
  it('the real screens tree has no screenT call outside its own namespace', () => {
    const files = import.meta.glob<string>('/src/screens/**/*.{ts,tsx,js,jsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    expect(checkScreenNamespaces(files)).toEqual([]);
  });
});
