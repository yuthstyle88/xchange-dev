import { createSignal, type Accessor } from 'solid-js';
import { t } from '../i18n';
import type { Key } from '../i18n/th';

const [soundOn, setSoundOn] = createSignal(false);
const [hintKey, setHintKey] = createSignal<Key>('queue.sound.hintInitial');

export { soundOn };
export const hint: Accessor<string> = () => t(hintKey());

let audioCtx: AudioContext | undefined;

function getAudioContextClass(): typeof AudioContext | undefined {
  if (typeof window !== 'undefined') {
    const w = window as unknown as {
      AudioContext?: typeof AudioContext;
      webkitAudioContext?: typeof AudioContext;
    };
    if (w.AudioContext) return w.AudioContext;
    if (w.webkitAudioContext) return w.webkitAudioContext;
  }
  if (typeof globalThis !== 'undefined') {
    const g = globalThis as unknown as {
      AudioContext?: typeof AudioContext;
      webkitAudioContext?: typeof AudioContext;
    };
    if (g.AudioContext) return g.AudioContext;
    if (g.webkitAudioContext) return g.webkitAudioContext;
  }
  return undefined;
}

export async function enable(): Promise<void> {
  const Ctor = getAudioContextClass();
  if (!Ctor) {
    throw new Error('AudioContext not supported');
  }
  if (!audioCtx) {
    audioCtx = new Ctor();
  }
  await audioCtx.resume();
  if (audioCtx.state !== 'running') {
    throw new Error('AudioContext failed to start');
  }
}

export function chime(): boolean {
  if (!audioCtx || audioCtx.state !== 'running') {
    return false;
  }
  try {
    const ctx = audioCtx;
    const now = ctx.currentTime;
    const freqs = [660, 880];
    for (let i = 0; i < freqs.length; i++) {
      const at = now + i * 0.19;
      const osc = ctx.createOscillator();
      osc.frequency.value = freqs[i];
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(ctx.destination);

      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(0.12, at + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.001, at + 0.3);

      osc.start(at);
      osc.stop(at + 0.31);
    }
    return true;
  } catch {
    return false;
  }
}

export async function toggleSound(): Promise<void> {
  if (soundOn()) {
    setSoundOn(false);
    setHintKey('queue.sound.hintOff');
  } else {
    try {
      await enable();
      setSoundOn(true);
      setHintKey('queue.sound.hintMuted');
      chime();
    } catch {
      setSoundOn(false);
      setHintKey('queue.sound.errPermission');
    }
  }
}

export async function testSound(): Promise<void> {
  let played = false;
  try {
    const Ctor = getAudioContextClass();
    if (Ctor) {
      if (!audioCtx) {
        audioCtx = new Ctor();
      }
      await audioCtx.resume();
      if (audioCtx.state === 'running') {
        played = chime();
      }
    }
  } catch {
    played = false;
  }

  if (played) {
    if (soundOn()) {
      setHintKey('queue.sound.testPlaying');
    } else {
      setHintKey('queue.sound.testMuted');
    }
  } else {
    setHintKey('queue.sound.errPlayback');
  }
}

const memoryNotes = new Map<string, string>();

async function withLock<T>(name: string, cb: () => T | Promise<T>): Promise<T> {
  if (
    typeof navigator !== 'undefined' &&
    navigator.locks &&
    typeof navigator.locks.request === 'function'
  ) {
    return await navigator.locks.request(name, cb);
  }
  return await cb();
}

export async function announce(
  state: { count: number; latest: string | null },
  userId: string,
  branch: string,
): Promise<void> {
  if (!soundOn()) return;
  if (state.count <= 0 || !state.latest) return;

  const storageKey = `xc.queue.sound.${userId}.${branch}`;
  const lockName = `xc.queue.sound.${branch}`;

  await withLock(lockName, async () => {
    let currentVal: string | null = null;
    let storageAvailable = true;

    try {
      if (typeof window !== 'undefined' && window.localStorage) {
        currentVal = window.localStorage.getItem(storageKey);
      }
    } catch {
      storageAvailable = false;
    }

    if (!storageAvailable) {
      currentVal = memoryNotes.get(storageKey) ?? null;
    }

    if (currentVal === state.latest) {
      return;
    }

    const played = chime();
    if (played) {
      if (storageAvailable) {
        try {
          window.localStorage.setItem(storageKey, state.latest!);
        } catch {
          memoryNotes.set(storageKey, state.latest!);
        }
      } else {
        memoryNotes.set(storageKey, state.latest!);
      }
    }
  });
}
