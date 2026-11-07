// XC Counter (docs/design/xc-counter-app.md §3, §8.6): the desk's view of `window.xcCounter`, which only the app's
// webview defines (`counter-app/src-tauri/src/bridge/init.js`). Skeleton by claude/bbe9ce8b; `bindCounter` is CA-5's.
import type { ApiClient } from '../api/client';
import { keyedWrite } from '../api/keyed-write';

export type ScanRefusal = 'noScanner' | 'busy' | 'timeout' | 'coverOpen' | 'device';

export interface CounterInfo {
  agentId: string;
  counterName: string;
  branchId: string;
  capabilities: { cashDrawer: boolean; scanner: boolean; kioskLock: boolean };
  appVersion: string;
  build: string;
}

export type ScanResult = { ok: true; file: Blob } | { ok: false; reason: ScanRefusal };

export interface XcCounter {
  info(): Promise<CounterInfo>;
  scanPassport(): Promise<ScanResult>;
  /** One nonce per call, signed by the server for this counter's agent (§5.4, §8.3). */
  bindingNonce(): Promise<string>;
}

declare global {
  interface Window {
    xcCounter?: XcCounter;
  }
}

/** §8.4. Hand-written until the generated type of `deskBindCounter` exists. */
export const DESK_BIND_COUNTER = '/api/v3/commands/desk_bind_counter';
export interface DeskBindCounterInput {
  nonce: string;
}
export interface DeskBindCounterAnswer {
  counterAgentId: string;
}

/** The app's bridge, or undefined in a plain browser. */
export function getXcCounter(): XcCounter | undefined {
  return window.xcCounter;
}

/**
 * CA-5: takes `bindingNonce()` once, then sends `deskBindCounter {nonce}` through `api` with `keyedWrite`, because the
 * staff cookie belongs to the page and not to Rust (§8.6). Answers true when bound. A refusal or a failure answers
 * false and never throws. No `xcCounter` answers false with no request.
 */
export async function bindCounter(api: ApiClient): Promise<boolean> {
  const bridge = getXcCounter();
  if (!bridge) return false;
  try {
    const nonce = await bridge.bindingNonce();
    await keyedWrite<DeskBindCounterInput, DeskBindCounterAnswer>(
      api,
      'POST',
      DESK_BIND_COUNTER,
    ).send({ nonce });
    return true;
  } catch {
    return false;
  }
}
