import type { ApiClient } from '../../shared/api/client';
import type {
  KioskScreenData,
  KioskScreenSaveInput,
} from '../../shared/api/generated/kiosk_screen';

export type CurrencyDisplay = 'SHOW' | 'ASK' | 'HIDE';

export const KIOSK_SCREEN_VIEW = 'kiosk_screen';
export const KIOSK_SCREEN_SAVE_PATH = '/api/v3/commands/kiosk_screen_save';

export interface ConfigurePayload {
  branchId: string;
  code: string;
  group?: string;
  display?: CurrencyDisplay;
  enabled?: boolean;
  buy?: string;
  sell?: string;
  previousUpdated?: string;
  partnerName?: string;
  partnerCost?: string | null;
}

export async function configure(
  api: Pick<ApiClient, 'post'>,
  p: ConfigurePayload,
): Promise<void> {
  await api.post('/api/kiosk', {
    action: 'configure',
    payload: p,
  });
}

export async function readKioskScreen(
  api: Pick<ApiClient, 'view'>,
  branch: string,
): Promise<KioskScreenData | undefined> {
  const res = await api.view<KioskScreenData>(KIOSK_SCREEN_VIEW, { branch });
  return res.data;
}

export async function saveKioskScreen(
  api: Pick<ApiClient, 'command'>,
  input: KioskScreenSaveInput,
  key = crypto.randomUUID(),
): Promise<KioskScreenData> {
  return api.command<KioskScreenSaveInput, KioskScreenData>(
    'POST',
    KIOSK_SCREEN_SAVE_PATH,
    input,
    key,
  );
}
