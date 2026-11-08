// OL-8 reuse-admin-list-states (owner round 13: "งาน css รียูส ด้วยนะ"): the loading box, retry button, error banner,
// table container and table base were copied byte-for-byte into each admin list screen (and home's table). One shared
// copy lives in list-states.css; each screen uses its class and drops its own copy. No visible change.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let read: (rel: string) => string = () => '';

function stripped(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = stripped(css);
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

function asObject(m: Map<string, string> | undefined): Record<string, string> | undefined {
  return m ? Object.fromEntries([...m.entries()].sort(([a], [b]) => a.localeCompare(b))) : undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  read = (rel) => fs.readFileSync(here + rel, 'utf8');
});

const SHARED = 'list-states.css';
const IMPORT = "import '../../../shared/ui/list-states.css';";

const LOADING = {
  'align-items': 'center',
  color: 'var(--xc-muted)',
  display: 'flex',
  'font-size': '1rem',
  'justify-content': 'center',
  padding: '3rem',
};
const RETRY = {
  'background-color': 'var(--xc-danger)',
  border: 'none',
  'border-radius': '0.375rem',
  color: 'var(--xc-on-brand)',
  cursor: 'pointer',
  'font-size': '0.875rem',
  'font-weight': '500',
  padding: '0.375rem 0.75rem',
};
const ERROR = {
  'align-items': 'center',
  'background-color': 'var(--xc-danger-soft)',
  border: '1px solid var(--xc-brand-200)',
  'border-radius': '0.5rem',
  color: 'var(--xc-danger)',
  display: 'flex',
  'justify-content': 'space-between',
  padding: '1rem 1.25rem',
};
const TABLE_WRAP = {
  'background-color': 'var(--xc-surface)',
  border: '1px solid var(--xc-line)',
  'border-radius': '0.5rem',
  'overflow-x': 'auto',
};
const TABLE = {
  'border-collapse': 'collapse',
  'font-size': '0.875rem',
  'text-align': 'left',
  width: '100%',
};

const CP = '../../screens/admin/campaign-performance/';
const CA = '../../screens/admin/campaigns/';
const NO = '../../screens/admin/notices/';
const RA = '../../screens/admin/rate-alert-settings/';
const SP = '../../screens/admin/starting-prices/';
const ME = '../../screens/admin/members/';
const HO = '../../screens/customer/home/';

describe('list-states shared rules', () => {
  it('LS-1 .xc-list-loading carries the loading box declarations', () => {
    expect(asObject(decls(read(SHARED), '.xc-list-loading'))).toEqual(LOADING);
  });
  it('LS-2 .xc-list-retry carries the retry button declarations', () => {
    expect(asObject(decls(read(SHARED), '.xc-list-retry'))).toEqual(RETRY);
  });
  it('LS-3 .xc-list-error carries the error banner declarations', () => {
    expect(asObject(decls(read(SHARED), '.xc-list-error'))).toEqual(ERROR);
  });
  it('LS-4 .xc-list-table-wrap carries the table container declarations', () => {
    expect(asObject(decls(read(SHARED), '.xc-list-table-wrap'))).toEqual(TABLE_WRAP);
  });
  it('LS-5 .xc-list-table carries the table base declarations', () => {
    expect(asObject(decls(read(SHARED), '.xc-list-table'))).toEqual(TABLE);
  });
});

describe('copied blocks are gone from each screen css', () => {
  const gone: [string, string][] = [
    [CP + 'campaign-performance.css', '.xc-cp-loading'],
    [CP + 'campaign-performance.css', '.xc-cp-error'],
    [CP + 'campaign-performance.css', '.xc-cp-retry-btn'],
    [CP + 'campaign-performance.css', '.xc-cp-table-container'],
    [CP + 'campaign-performance.css', '.xc-cp-table'],
    [CA + 'campaigns.css', '.xc-campaigns-loading'],
    [CA + 'campaigns.css', '.xc-campaigns-error'],
    [CA + 'campaigns.css', '.xc-campaigns-retry-btn'],
    [CA + 'campaigns.css', '.xc-campaigns-table-container'],
    [CA + 'campaigns.css', '.xc-campaigns-table'],
    [NO + 'notices.css', '.admin-notices-loading'],
    [NO + 'notices.css', '.admin-notices-error'],
    [NO + 'notices.css', '.admin-notices-retry-btn'],
    [RA + 'rate-alert-settings.css', '.rate-alert-loading'],
    [RA + 'rate-alert-settings.css', '.rate-alert-retry-btn'],
    [SP + 'starting-prices.css', '.xc-sp-loading'],
    [SP + 'starting-prices.css', '.xc-sp-retry-btn'],
    [SP + 'starting-prices.css', '.xc-sp-table-scroll'],
    [SP + 'starting-prices.css', '.xc-sp-table'],
    [HO + 'home.css', '.xc-home-table'],
  ];
  it.each(gone)('LS-6 %s has no %s rule', (file, selector) => {
    expect(decls(read(file), selector)).toBeUndefined();
  });

  it('LS-7 the rules that differ stay screen-local', () => {
    expect(decls(read(CP + 'campaign-performance.css'), '.xc-cp-retry-btn:hover')?.get('background-color')).toBe(
      'var(--xc-brand-strong)',
    );
    expect(decls(read(RA + 'rate-alert-settings.css'), '.rate-alert-error')?.get('border')).toBe(
      '1px solid var(--xc-danger)',
    );
    expect(decls(read(SP + 'starting-prices.css'), '.xc-sp-error')?.get('border')).toBe('1px solid var(--xc-danger)');
    expect(decls(read(CP + 'campaign-performance.css'), '.xc-cp-table th')).toBeDefined();
    expect(decls(read(CA + 'campaigns.css'), '.xc-campaigns-table th')).toBeDefined();
    expect(decls(read(SP + 'starting-prices.css'), '.xc-sp-table th')).toBeDefined();
    expect(decls(read(HO + 'home.css'), '.xc-home-table th')).toBeDefined();
  });
});

describe('each screen uses the shared classes', () => {
  it('LS-8 campaign performance', () => {
    const tsx = read(CP + 'CampaignPerformance.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain('<div class="xc-list-error">');
    expect(tsx).toContain('class="xc-list-retry xc-cp-retry-btn"');
    expect(tsx).toContain(`<div class="xc-list-loading">{t('loading')}</div>`);
    expect(tsx).toContain('<div class="xc-list-table-wrap">');
    expect(tsx).toContain('<table class="xc-list-table xc-cp-table">');
    expect(tsx).not.toMatch(/xc-cp-(loading|error|table-container)\b/);
  });

  it('LS-9 campaigns', () => {
    const tsx = read(CA + 'Campaigns.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx.split('<div class="xc-list-error">').length - 1).toBe(3);
    expect(tsx).toContain('class="xc-list-retry"');
    expect(tsx.split(`<div class="xc-list-loading">{t('loading')}</div>`).length - 1).toBe(2);
    expect(tsx).toContain('<div class="xc-list-table-wrap">');
    expect(tsx).toContain('<table class="xc-list-table xc-campaigns-table">');
    expect(tsx).not.toMatch(/xc-campaigns-(loading|error|retry-btn|table-container)\b/);
  });

  it('LS-10 members (borrowed the campaigns classes)', () => {
    const tsx = read(ME + 'Members.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain('<div class="xc-list-error" role="alert">');
    expect(tsx).toContain('<div class="xc-list-table-wrap">');
    expect(tsx).toContain('<table class="xc-list-table xc-campaigns-table">');
    expect(tsx).not.toMatch(/xc-campaigns-(error|table-container)\b/);
  });

  it('LS-11 notices', () => {
    const tsx = read(NO + 'Notices.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain(`<div class="xc-list-loading">{t('loading')}</div>`);
    expect(tsx).toContain('<div class="xc-list-error">');
    expect(tsx).toContain('<button type="button" class="xc-list-retry" onClick={refetch}>');
    expect(tsx).not.toMatch(/admin-notices-(loading|error|retry-btn)\b/);
  });

  it('LS-12 rate alert settings', () => {
    const tsx = read(RA + 'RateAlertSettings.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain('<div class="xc-list-loading" data-state="loading">');
    expect(tsx).toContain('class="xc-list-retry"');
    expect(tsx).toContain('<div class="rate-alert-error" data-state="error" role="alert">');
    expect(tsx).not.toMatch(/rate-alert-(loading|retry-btn)\b/);
  });

  it('LS-13 starting prices', () => {
    const tsx = read(SP + 'StartingPrices.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain(`<div class="xc-list-loading">{t('loading')}</div>`);
    expect(tsx).toContain('<button type="button" class="xc-list-retry" onClick={() => void loadView(false)}>');
    expect(tsx).toContain('<div class="xc-sp-error" role="alert">');
    expect(tsx).toContain('<div class="xc-list-table-wrap">');
    expect(tsx).toContain('<table class="xc-list-table xc-sp-table">');
    expect(tsx).not.toMatch(/xc-sp-(loading|retry-btn|table-scroll)\b/);
  });

  it('LS-14 customer home (table base only)', () => {
    const tsx = read(HO + 'HomeScreen.tsx');
    expect(tsx).toContain(IMPORT);
    expect(tsx).toContain('<table class="xc-list-table xc-home-table">');
  });

  it('LS-15 the shared import sits right before each screen stylesheet import', () => {
    const pairs: [string, string][] = [
      [CP + 'CampaignPerformance.tsx', "import './campaign-performance.css';"],
      [CA + 'Campaigns.tsx', "import './campaigns.css';"],
      [ME + 'Members.tsx', "import './members.css';"],
      [NO + 'Notices.tsx', "import './notices.css';"],
      [RA + 'RateAlertSettings.tsx', "import './rate-alert-settings.css';"],
      [SP + 'StartingPrices.tsx', "import './starting-prices.css';"],
      [HO + 'HomeScreen.tsx', "import './home.css';"],
    ];
    for (const [file, screenImport] of pairs) {
      expect(read(file), file).toContain(`${IMPORT}\n${screenImport}`);
    }
  });
});
