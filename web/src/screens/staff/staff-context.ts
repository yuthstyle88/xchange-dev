import { batch, createSignal, onCleanup, onMount, type Accessor } from 'solid-js';
import { ApiClient, ApiError } from '../../shared/api/client';
import { wordedError } from '../../shared/i18n/worded-error';
import { onFreshNotice } from '../../shared/ui/notice-bell/notice-feed';
import type { StaffCtx, StaffFloor, StaffMe } from './cards';

export interface StaffContext {
  me: Accessor<StaffMe | undefined>;
  floor: Accessor<StaffFloor | undefined>;
  error: Accessor<string | undefined>;
  ctx: Accessor<StaffCtx | undefined>;
  refresh(): Promise<void>;
}

export function createStaffContext(opts: {
  api: ApiClient;
  branch?: Accessor<string>;
  signInUrl: string;
  navigate(url: string): void;
}): StaffContext {
  const [me, setMe] = createSignal<StaffMe | undefined>(undefined);
  const [floor, setFloor] = createSignal<StaffFloor | undefined>(undefined);
  const [error, setError] = createSignal<string | undefined>(undefined);

  let seq = 0;

  const refresh = async (): Promise<void> => {
    const mine = ++seq;
    const q = opts.branch ? { branch: opts.branch() } : {};
    let meData: StaffMe | undefined;
    try {
      const meRes = await opts.api.view<StaffMe>('staff_me', q);
      if (mine !== seq) return;
      meData = meRes.data;
    } catch (err: unknown) {
      if (mine !== seq) return;
      if (err instanceof ApiError && err.status === 401 && err.code === 'signIn') {
        opts.navigate(opts.signInUrl);
        return;
      }
      setError(wordedError(err));
      return;
    }

    let floorData: StaffFloor | undefined;
    let floorErr: string | undefined;
    try {
      const floorRes = await opts.api.view<StaffFloor>('staff_floor', q);
      if (mine !== seq) return;
      floorData = floorRes.data;
    } catch (err: unknown) {
      if (mine !== seq) return;
      floorErr = wordedError(err);
    }

    batch(() => {
      if (meData) {
        setMe(meData);
      }
      if (floorData !== undefined) {
        setFloor(floorData);
      }
      setError(floorErr);
    });
  };

  onMount(() => {
    onCleanup(onFreshNotice('cash_count', () => void refresh()));
  });

  const ctx: Accessor<StaffCtx | undefined> = () => {
    const currentMe = me();
    return currentMe ? { me: currentMe, floor: floor(), refresh } : undefined;
  };

  return { me, floor, error, ctx, refresh };
}
