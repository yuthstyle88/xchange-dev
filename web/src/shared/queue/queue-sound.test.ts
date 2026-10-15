// V1-queue-card (#487): the page's queue sound, design docs/design/xchange-v3-web.md §12.18.6 rows
// C7–C9 and §12.18.8 (v2: src/devices-public/account-bar.js:32-38 of xchange-v2-source@4d0bdbb).
// Written before the implementation by the lane controller. Do not edit, delete or weaken; add
// tests in a file of your own.
//
// The API this file pins, `web/src/shared/queue/queue-sound.ts` — one sound per page, so these are
// module-level exports, not a factory (ticket §7):
//
//   export const soundOn: Accessor<boolean>;
//   export const hint: Accessor<string>;              // the hint text, in the current language
//   export function enable(): Promise<void>;          // account-bar.js:33 enableAudio
//   export function chime(): boolean;                 // :34 (C8)
//   export function announce(state: { count: number; latest: string | null },
//                            userId: string, branch: string): Promise<void>;   // :37 (C9)
//
// Sound is switched on the way a person does it: by pressing `เปิดเสียงแจ้งเตือน` in
// `QueueSoundControls` (web/src/shared/ui/QueueSoundControls.tsx, no props), which renders
// the buttons of C7 over this module. The controls' own texts are tested in QueueCard.test.tsx.
//
// "A page" is one fresh copy of the module graph (`vi.resetModules()` then `import()`), so two
// pages ("tabs") are two copies alive at once that share this window's `localStorage` and
// `navigator.locks` — the two things v2 used to keep one chime per new request across tabs.
// `AudioContext`, `localStorage` and `navigator.locks` are fakes this file installs.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createComponent } from 'solid-js';
import { render } from 'solid-js/web';
import { warmModules } from '../testing/warm-modules';

// ── fake AudioContext ────────────────────────────────────────────────────────

interface Ramp {
  kind: 'set' | 'linear' | 'exp';
  value: number;
  at: number;
}

interface FakeOscillator {
  frequency: { value: number };
  connectedTo: unknown[];
  startAt: number | undefined;
  stopAt: number | undefined;
  /** Which sound locks were held when this oscillator was made. */
  locksHeld: string[];
}

interface FakeGain {
  gain: { ramps: Ramp[] };
  connectedTo: unknown[];
}

const audio = {
  /** What `resume()` leaves the context in. */
  resumeTo: 'running' as string,
  contexts: [] as FakeAudioContext[],
  oscillators: [] as FakeOscillator[],
  gains: [] as FakeGain[],
  /** One chime = one 660 Hz oscillator (C8). */
  chimes(): number {
    return audio.oscillators.filter((o) => o.frequency.value === 660).length;
  },
  reset() {
    audio.resumeTo = 'running';
    audio.contexts = [];
    audio.oscillators = [];
    audio.gains = [];
  },
};

class FakeAudioContext {
  state = 'suspended';
  currentTime = 5;
  readonly destination = { node: 'destination' };
  resumes = 0;
  constructor() {
    audio.contexts.push(this);
  }
  async resume(): Promise<void> {
    this.resumes += 1;
    this.state = audio.resumeTo;
  }
  createOscillator(): FakeOscillator & { connect(n: unknown): void; start(at: number): void; stop(at: number): void } {
    const osc = {
      frequency: { value: 0 },
      connectedTo: [] as unknown[],
      startAt: undefined as number | undefined,
      stopAt: undefined as number | undefined,
      locksHeld: [...locks.held],
      connect(node: unknown) {
        osc.connectedTo.push(node);
      },
      start(at: number) {
        osc.startAt = at;
      },
      stop(at: number) {
        osc.stopAt = at;
      },
    };
    audio.oscillators.push(osc);
    return osc;
  }
  createGain(): FakeGain & { connect(n: unknown): void } {
    const ramps: Ramp[] = [];
    const g = {
      gain: {
        ramps,
        setValueAtTime(value: number, at: number) {
          ramps.push({ kind: 'set', value, at });
        },
        linearRampToValueAtTime(value: number, at: number) {
          ramps.push({ kind: 'linear', value, at });
        },
        exponentialRampToValueAtTime(value: number, at: number) {
          ramps.push({ kind: 'exp', value, at });
        },
      },
      connectedTo: [] as unknown[],
      connect(node: unknown) {
        g.connectedTo.push(node);
      },
    };
    audio.gains.push(g);
    return g;
  }
}

// ── fake navigator.locks ─────────────────────────────────────────────────────

const locks = {
  held: new Set<string>(),
  requests: [] as string[],
  waiters: [] as Array<() => void>,
  async request<T>(name: string, cb: (lock: { name: string }) => T | Promise<T>): Promise<T> {
    locks.requests.push(name);
    while (locks.held.has(name)) {
      await new Promise<void>((resolve) => locks.waiters.push(resolve));
    }
    locks.held.add(name);
    try {
      // a real lock manager answers in a later task, never synchronously
      await new Promise((r) => setTimeout(r, 0));
      return await cb({ name });
    } finally {
      locks.held.delete(name);
      const waiting = locks.waiters.splice(0);
      waiting.forEach((w) => w());
    }
  },
  reset() {
    locks.held.clear();
    locks.requests = [];
    locks.waiters = [];
  },
};

// ── fake localStorage ────────────────────────────────────────────────────────

class FakeStorage {
  readonly map = new Map<string, string>();
  /** When set, getItem and setItem throw (private mode, blocked site data). */
  broken = false;
  get length(): number {
    return this.map.size;
  }
  key(i: number): string | null {
    return [...this.map.keys()][i] ?? null;
  }
  getItem(key: string): string | null {
    if (this.broken) throw new DOMException('blocked', 'SecurityError');
    return this.map.has(key) ? (this.map.get(key) as string) : null;
  }
  setItem(key: string, value: string): void {
    if (this.broken) throw new DOMException('blocked', 'SecurityError');
    this.map.set(key, String(value));
  }
  removeItem(key: string): void {
    this.map.delete(key);
  }
  clear(): void {
    this.map.clear();
  }
}

let storage: FakeStorage;

function install(opts: { locks?: boolean; audio?: 'AudioContext' | 'webkitAudioContext' | 'none' } = {}) {
  storage = new FakeStorage();
  vi.stubGlobal('localStorage', storage);
  Object.defineProperty(window, 'localStorage', { value: storage, configurable: true });
  if (opts.locks !== false) {
    Object.defineProperty(navigator, 'locks', { value: locks, configurable: true });
  }
  const kind = opts.audio ?? 'AudioContext';
  if (kind !== 'none') {
    vi.stubGlobal(kind, FakeAudioContext);
    Object.defineProperty(window, kind, { value: FakeAudioContext, configurable: true, writable: true });
  }
}

function uninstall() {
  vi.unstubAllGlobals();
  for (const name of ['AudioContext', 'webkitAudioContext', 'localStorage'] as const) {
    if (Object.getOwnPropertyDescriptor(window, name)?.configurable) {
      delete (window as unknown as Record<string, unknown>)[name];
    }
  }
  delete (navigator as unknown as Record<string, unknown>).locks;
}

// ── pages ────────────────────────────────────────────────────────────────────

type SoundModule = typeof import('./queue-sound');

interface Page {
  sound: SoundModule;
  /** Presses a button of this page's QueueSoundControls by its visible text. */
  press(text: string): Promise<void>;
  dispose(): void;
}

const disposers: Array<() => void> = [];

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** A fresh page: its own queue-sound module and its own controls. */
async function openPage(): Promise<Page> {
  vi.resetModules();
  const sound = await import('./queue-sound');
  const { QueueSoundControls } = await import('../ui/QueueSoundControls');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const dispose = render(() => createComponent(QueueSoundControls, {}), host);
  disposers.push(dispose);
  return {
    sound,
    async press(text: string) {
      const button = Array.from(host.querySelectorAll('button')).find(
        (b) => (b.textContent ?? '').trim() === text,
      );
      if (!button) throw new Error(`no button ${JSON.stringify(text)} in ${JSON.stringify(host.textContent)}`);
      button.click();
      await settle();
    },
    dispose,
  };
}

/** A page with sound switched on (C7), the switch-on chime already played. */
async function pageWithSound(): Promise<Page> {
  const page = await openPage();
  await page.press(SOUND_ON);
  expect(page.sound.soundOn()).toBe(true);
  return page;
}

const SOUND_ON = 'เปิดเสียงแจ้งเตือน';
const HINT_FIRST = 'กดเปิดเสียงครั้งแรกเพื่อให้เบราว์เซอร์อนุญาต';

const waiting = (latest: string | null, count = 1) => ({ count, latest });

beforeAll(
  () =>
    warmModules(
      () => import('./queue-sound'),
      () => import('../ui/QueueSoundControls'),
      () => import('../i18n'),
      () => import('../i18n/en'),
    ),
  60_000,
);

beforeEach(() => {
  audio.reset();
  locks.reset();
  install();
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  uninstall();
});

// ── tests ────────────────────────────────────────────────────────────────────

describe('queue-sound: a page starts silent (C7)', () => {
  it('sound is off and the hint asks for the first press; nothing touches audio on import', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    expect(sound.soundOn()).toBe(false);
    expect(sound.hint()).toBe(HINT_FIRST);
    expect(audio.contexts).toHaveLength(0);
    expect(sound.chime()).toBe(false);
    expect(audio.oscillators).toHaveLength(0);
  });

  it('the hint follows the language (W10)', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    const i18n = await import('../i18n');
    const { en } = await import('../i18n/en');
    i18n.setLang('en');
    expect(sound.hint()).toBe(en['queue.sound.hintInitial']);
    i18n.setLang('th');
    expect(sound.hint()).toBe(HINT_FIRST);
  });
});

describe('queue-sound: enable() (account-bar.js:33)', () => {
  it('makes one AudioContext, resumes it and resolves once it runs; a second call reuses it', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    await sound.enable();
    expect(audio.contexts).toHaveLength(1);
    expect(audio.contexts[0].resumes).toBe(1);
    expect(audio.contexts[0].state).toBe('running');
    await sound.enable();
    expect(audio.contexts).toHaveLength(1);
    expect(audio.contexts[0].resumes).toBe(2);
  });

  it('rejects when the context does not reach running, and sound stays off', async () => {
    audio.resumeTo = 'suspended';
    vi.resetModules();
    const sound = await import('./queue-sound');
    await expect(sound.enable()).rejects.toBeDefined();
    expect(sound.soundOn()).toBe(false);
    expect(sound.chime()).toBe(false);
    expect(audio.oscillators).toHaveLength(0);
  });

  it('rejects when the browser has no AudioContext at all', async () => {
    uninstall();
    install({ audio: 'none' });
    vi.resetModules();
    const sound = await import('./queue-sound');
    await expect(sound.enable()).rejects.toBeDefined();
    expect(sound.chime()).toBe(false);
  });

  it('falls back to webkitAudioContext (account-bar.js:33)', async () => {
    uninstall();
    install({ audio: 'webkitAudioContext' });
    vi.resetModules();
    const sound = await import('./queue-sound');
    await sound.enable();
    expect(audio.contexts).toHaveLength(1);
    expect(sound.chime()).toBe(true);
  });
});

describe('queue-sound: chime() (C8, account-bar.js:34)', () => {
  it('is false and plays nothing before audio is enabled', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    expect(sound.chime()).toBe(false);
    expect(audio.oscillators).toHaveLength(0);
  });

  it('660 Hz then 880 Hz, 0.19 s apart, gain 0 → 0.12 in 20 ms → 0.001 at 0.3 s, stopped at 0.31 s', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    await sound.enable();
    const ctx = audio.contexts[0];
    expect(sound.chime()).toBe(true);

    expect(audio.oscillators.map((o) => o.frequency.value)).toEqual([660, 880]);
    expect(audio.gains).toHaveLength(2);
    [0, 1].forEach((i) => {
      const at = ctx.currentTime + i * 0.19;
      const osc = audio.oscillators[i];
      const gain = audio.gains[i];
      expect(osc.startAt).toBeCloseTo(at, 9);
      expect(osc.stopAt).toBeCloseTo(at + 0.31, 9);
      expect(osc.connectedTo).toEqual([gain]);
      expect(gain.connectedTo).toEqual([ctx.destination]);
      const ramps = gain.gain.ramps;
      expect(ramps.map((r) => r.kind)).toEqual(['set', 'linear', 'exp']);
      expect(ramps[0].value).toBe(0);
      expect(ramps[0].at).toBeCloseTo(at, 9);
      expect(ramps[1].value).toBeCloseTo(0.12, 9);
      expect(ramps[1].at).toBeCloseTo(at + 0.02, 9);
      expect(ramps[2].value).toBeCloseTo(0.001, 9);
      expect(ramps[2].at).toBeCloseTo(at + 0.3, 9);
    });
  });

  it('is false again when the browser suspends the context later', async () => {
    vi.resetModules();
    const sound = await import('./queue-sound');
    await sound.enable();
    audio.contexts[0].state = 'suspended';
    expect(sound.chime()).toBe(false);
    expect(audio.oscillators).toHaveLength(0);
  });
});

describe('queue-sound: announce() — one chime per new request (C9, account-bar.js:37-38)', () => {
  it('sound off: no chime, nothing stored, no lock asked for', async () => {
    const page = await openPage();
    await page.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(0);
    expect(storage.map.has('xc.queue.sound.E1.B1')).toBe(false);
    expect(locks.requests).toEqual([]);
  });

  it('nothing waiting (count 0) or no latest: no chime and nothing stored', async () => {
    const page = await pageWithSound();
    const before = audio.chimes();
    await page.sound.announce(waiting('kq-1', 0), 'E1', 'B1');
    await page.sound.announce(waiting(null, 2), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before);
    expect(storage.map.has('xc.queue.sound.E1.B1')).toBe(false);
  });

  it('no latest after a stored one: no chime, and the stored latest stays', async () => {
    const page = await pageWithSound();
    await page.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    const before = audio.chimes();
    await page.sound.announce(waiting(null, 2), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-1');
  });

  it('a new latest: chimes under the lock xc.queue.sound.<branch> and stores it at xc.queue.sound.<user>.<branch>', async () => {
    const page = await pageWithSound();
    const before = audio.chimes();
    await page.sound.announce(waiting('kq-1', 2), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);
    expect(locks.requests).toEqual(['xc.queue.sound.B1']);
    const played = audio.oscillators.filter((o) => o.frequency.value === 660).at(-1);
    expect(played?.locksHeld).toEqual(['xc.queue.sound.B1']);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-1');
  });

  it('the same latest again on the same page: no second chime', async () => {
    const page = await pageWithSound();
    const before = audio.chimes();
    await page.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await page.sound.announce(waiting('kq-1', 3), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);
  });

  it('two tabs, the same latest: one chime between them; the next latest: one chime again', async () => {
    const tabA = await pageWithSound();
    const tabB = await pageWithSound();
    expect(tabA.sound).not.toBe(tabB.sound);
    const before = audio.chimes();

    await tabA.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await tabB.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);

    await tabB.sound.announce(waiting('kq-2', 2), 'E1', 'B1');
    await tabA.sound.announce(waiting('kq-2', 2), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 2);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-2');
  });

  it('two tabs announcing the same latest at the same moment: still one chime (the lock serialises them)', async () => {
    const tabA = await pageWithSound();
    const tabB = await pageWithSound();
    const before = audio.chimes();
    await Promise.all([
      tabA.sound.announce(waiting('kq-9'), 'E1', 'B1'),
      tabB.sound.announce(waiting('kq-9'), 'E1', 'B1'),
    ]);
    await settle();
    expect(audio.chimes()).toBe(before + 1);
    expect(locks.requests).toEqual(['xc.queue.sound.B1', 'xc.queue.sound.B1']);
  });

  it('the key carries the user and the branch: another user, or another branch, still hears it', async () => {
    const tabA = await pageWithSound();
    const tabB = await pageWithSound();
    const before = audio.chimes();
    await tabA.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await tabB.sound.announce(waiting('kq-1'), 'E2', 'B1');
    await tabB.sound.announce(waiting('kq-1'), 'E1', 'B2');
    await settle();
    expect(audio.chimes()).toBe(before + 3);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-1');
    expect(storage.map.get('xc.queue.sound.E2.B1')).toBe('kq-1');
    expect(storage.map.get('xc.queue.sound.E1.B2')).toBe('kq-1');
    expect(locks.requests).toEqual(['xc.queue.sound.B1', 'xc.queue.sound.B1', 'xc.queue.sound.B2']);
  });

  it('a browser without navigator.locks: still one chime per latest across tabs, through localStorage', async () => {
    uninstall();
    install({ locks: false });
    expect('locks' in navigator).toBe(false);
    const tabA = await pageWithSound();
    const tabB = await pageWithSound();
    const before = audio.chimes();
    await tabA.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await tabB.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-1');
  });

  it('localStorage throws: one chime per latest per page', async () => {
    const tabA = await pageWithSound();
    const tabB = await pageWithSound();
    storage.broken = true;
    const before = audio.chimes();

    await tabA.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await tabA.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);

    // the other page cannot see the first page's note, so it chimes once too
    await tabB.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await tabB.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 2);

    await tabA.sound.announce(waiting('kq-2'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 3);
  });

  it('a chime that could not play is not stored, so the request is announced once audio runs again', async () => {
    const page = await pageWithSound();
    const before = audio.chimes();
    audio.contexts[0].state = 'suspended';
    await page.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before);
    expect(storage.map.has('xc.queue.sound.E1.B1')).toBe(false);

    audio.contexts[0].state = 'running';
    await page.sound.announce(waiting('kq-1'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before + 1);
    expect(storage.map.get('xc.queue.sound.E1.B1')).toBe('kq-1');
  });

  it('sound switched off again: a new latest is not announced', async () => {
    const page = await pageWithSound();
    await page.press('เสียงเปิด · กดปิด');
    expect(page.sound.soundOn()).toBe(false);
    const before = audio.chimes();
    await page.sound.announce(waiting('kq-5'), 'E1', 'B1');
    await settle();
    expect(audio.chimes()).toBe(before);
    expect(storage.map.has('xc.queue.sound.E1.B1')).toBe(false);
  });
});
