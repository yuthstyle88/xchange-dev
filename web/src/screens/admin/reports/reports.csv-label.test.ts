// OL-7 row 61 (verification of 2026-11-07, sheet row-061): the reports tabs' CSV buttons sit in the same place on every
// tab, but tabs 1-2 say "ส่งออกรายการ CSV" and the employee tab said "ดาวน์โหลด CSV". Every CSV export in the admin
// (overview, transactions, both report tabs) reads `exportCsv`, so the employee tab's `#employee-csv` reads it too and
// the one-off `downloadCsv` key goes. The rendered label is pinned in Reports.test.tsx / Reports.extra.test.tsx /
// Reports.backdated.test.tsx / export-permission.test.tsx (restated on this branch).
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';
import { en, th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let source = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  source = fs.readFileSync(here + 'Reports.tsx', 'utf8');
});

describe('reports: one CSV label on every tab', () => {
  it('RL-1 the downloadCsv key is gone from both tables', () => {
    expect(Object.keys(th)).not.toContain('downloadCsv');
    expect(Object.keys(en)).not.toContain('downloadCsv');
  });

  it("RL-2 the #employee-csv button's label is t('exportCsv')", () => {
    const at = source.indexOf('id="employee-csv"');
    expect(at, '#employee-csv in Reports.tsx').toBeGreaterThan(0);
    const button = source.slice(at, source.indexOf('</button>', at));
    expect(button).toContain("t('exportCsv')");
    expect(source).not.toContain("t('downloadCsv')");
  });

  it('RL-3 the shared label reads ส่งออกรายการ CSV / Export CSV', () => {
    expect(th.exportCsv).toBe('ส่งออกรายการ CSV');
    expect(en.exportCsv).toBe('Export CSV');
  });
});
