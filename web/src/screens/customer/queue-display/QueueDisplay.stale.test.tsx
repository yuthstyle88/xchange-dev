import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import * as QueueDisplayMod from './QueueDisplay';
import { QueueDisplay } from './QueueDisplay';
import * as i18nMod from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

function parseRules(css: string): Rule[] {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const idx = d.indexOf(':');
      if (idx > 0) decls.set(d.slice(0, idx).trim().toLowerCase(), d.slice(idx + 1).trim());
    }
    rules.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), decls });
  }
  return rules;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const FIRST = {
  version: 7,
  changed: true,
  data: {
    called: [{ displayNumber: 'K042', status: 'CALLED', calledAt: '2026-10-21T10:00:00Z' }],
    recent: [{ displayNumber: 'K041', calledAt: '2026-10-21T09:58:00Z' }],
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let hostAnswers: Array<() => Promise<Response> | Response>;
let displayAnswers: Array<() => Promise<Response> | Response>;

const fetchFn = vi.fn(async (url: string) => {
  if (url === '/api/public/host') {
    const next = hostAnswers.shift();
    return next ? next() : json(200, { org: 'ORG_1' });
  }
  const next = displayAnswers.shift();
  return next ? next() : json(200, { version: 7, changed: false });
});

async function tick(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}

function mount(): void {
  dispose = render(() => <QueueDisplay fetch={fetchFn} search="?branch=MAIN" intervalMs={5_000} />, host);
}

const statusEl = () => host.querySelector<HTMLElement>('.queue-display-status');
const calledNumbers = () => [...host.querySelectorAll('[data-called]')].map((e) => e.textContent?.trim());
const emptyEls = () => [...host.querySelectorAll('.queue-display-empty')];

describe('QueueDisplay: connection and stale-data status (#6556)', () => {
  let rules: Rule[];
  const rule = (selector: string) => rules.find((r) => r.selector === selector);

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const css = fs.readFileSync(here + 'queue-display.css', 'utf8');
    rules = parseRules(css);
  });

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-11-10T07:05:00Z')); // 14:05 Asia/Bangkok
    setAudience('customer');
    setLang('en');
    hostAnswers = [];
    displayAnswers = [() => json(200, FIRST)];
    fetchFn.mockClear();
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
  });

  it('exports connectionAfter pure helper with loading/ok/stale state transitions', () => {
    const connectionAfter = (
      QueueDisplayMod as unknown as {
        connectionAfter?: (
          prev: 'loading' | 'ok' | 'stale',
          outcome: 'ok' | 'fail',
          failures: number,
        ) => { connection: 'loading' | 'ok' | 'stale'; failures: number };
      }
    ).connectionAfter;
    expect(typeof connectionAfter).toBe('function');
    expect(connectionAfter!('loading', 'fail', 0)).toEqual({ connection: 'loading', failures: 1 });
    expect(connectionAfter!('loading', 'fail', 1)).toEqual({ connection: 'stale', failures: 2 });
    expect(connectionAfter!('loading', 'ok', 1)).toEqual({ connection: 'ok', failures: 0 });
    expect(connectionAfter!('ok', 'fail', 0)).toEqual({ connection: 'ok', failures: 1 });
    expect(connectionAfter!('ok', 'fail', 1)).toEqual({ connection: 'stale', failures: 2 });
    expect(connectionAfter!('stale', 'fail', 2)).toEqual({ connection: 'stale', failures: 3 });
    expect(connectionAfter!('stale', 'ok', 3)).toEqual({ connection: 'ok', failures: 0 });
  });

  it('defines connecting, stale, lastUpdated, neverUpdated in th/en/zh', () => {
    const th = i18nMod.th as Record<string, string>;
    const en = i18nMod.en as Record<string, string>;
    const zh = i18nMod.zh as Record<string, string>;
    expect(th.connecting).toBe('กำลังเชื่อมต่อ…');
    expect(en.connecting).toBe('Connecting…');
    expect(zh.connecting).toBe('正在连接…');
    expect(th.stale).toBe('ขาดการเชื่อมต่อ ข้อมูลอาจไม่เป็นปัจจุบัน');
    expect(en.stale).toBe('Connection lost, numbers may be out of date');
    expect(zh.stale).toBe('连接中断，信息可能不是最新');
    expect(th.lastUpdated).toBe('อัปเดตล่าสุด');
    expect(en.lastUpdated).toBe('Last updated');
    expect(zh.lastUpdated).toBe('最后更新');
    expect(th.neverUpdated).toBe('ยังไม่ได้รับข้อมูล');
    expect(en.neverUpdated).toBe('No data received yet');
    expect(zh.neverUpdated).toBe('尚未收到数据');
  });

  it('First poll pending: shows Connecting… directly under .queue-display-toolbar and no empty text', async () => {
    let resolvePoll!: (r: Response) => void;
    hostAnswers = [
      () =>
        new Promise<Response>((res) => {
          resolvePoll = res;
        }),
    ];
    displayAnswers = [() => json(200, { version: 1, changed: true, data: { called: [], recent: [] } })];
    mount();
    const toolbar = host.querySelector('.queue-display-toolbar');
    const st = statusEl();
    expect(st).not.toBeNull();
    expect(toolbar?.nextElementSibling).toBe(st);
    expect(st?.getAttribute('role')).toBe('status');
    expect(st?.getAttribute('aria-live')).toBe('polite');
    expect(st?.getAttribute('data-connection')).toBe('loading');
    expect(st?.textContent).toBe('Connecting…');
    expect(emptyEls()).toHaveLength(0);

    resolvePoll(json(200, { org: 'ORG_1' }));
    await tick(0);
    expect(st?.getAttribute('data-connection')).toBe('ok');
    expect(st?.textContent).toBe('');
    expect(emptyEls()).toHaveLength(2);
  });

  it('First poll fails twice: shows stale with neverUpdated and no empty text', async () => {
    const fail = () => json(503, { error: { code: 'down' } });
    hostAnswers = [fail, fail];
    mount();
    await tick(0);
    expect(statusEl()?.getAttribute('data-connection')).toBe('loading');
    expect(emptyEls()).toHaveLength(0);

    await tick(5_000);
    expect(statusEl()?.getAttribute('data-connection')).toBe('stale');
    expect(statusEl()?.textContent).toBe('Connection lost, numbers may be out of date No data received yet');
    expect(emptyEls()).toHaveLength(0);
  });

  it('Success, then 1 failure keeps status empty; ≥2 failures shows stale with Bangkok HH:mm; recovery clears it', async () => {
    const fail = () => json(500, { error: { code: 'boom' } });
    displayAnswers = [
      () => json(200, FIRST),
      fail,
      fail,
      () => json(200, { version: 7, changed: false }),
    ];
    mount();
    await tick(0);
    expect(statusEl()?.getAttribute('data-connection')).toBe('ok');
    expect(statusEl()?.textContent).toBe('');
    expect(calledNumbers()).toEqual(['K042']);

    // 1st failure: one blip does not flash the banner
    await tick(5_000);
    expect(statusEl()?.getAttribute('data-connection')).toBe('ok');
    expect(statusEl()?.textContent).toBe('');
    expect(calledNumbers()).toEqual(['K042']);

    // 2nd failure: stale banner appears with 14:05 (07:05Z in Asia/Bangkok), numbers stay visible
    await tick(5_000);
    expect(statusEl()?.getAttribute('data-connection')).toBe('stale');
    expect(statusEl()?.textContent).toBe('Connection lost, numbers may be out of date Last updated 14:05');
    expect(calledNumbers()).toEqual(['K042']);

    // Recovery (changed: false counts as success)
    await tick(5_000);
    expect(statusEl()?.getAttribute('data-connection')).toBe('ok');
    expect(statusEl()?.textContent).toBe('');
    expect(calledNumbers()).toEqual(['K042']);
  });

  it('styles .queue-display-status with danger tokens and hides :empty', () => {
    const st = rule('.queue-display-status');
    expect(st).toBeDefined();
    expect(st?.decls.get('background') ?? st?.decls.get('background-color')).toContain('var(--xc-danger-soft)');
    expect(st?.decls.get('color')).toBe('var(--xc-danger)');
    expect(st?.decls.get('border') ?? st?.decls.get('border-color')).toContain('var(--xc-danger)');
    expect(st?.decls.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(rule('.queue-display-status:empty')?.decls.get('display')).toBe('none');
  });
});
