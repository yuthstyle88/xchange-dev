import type { ApiClient } from '../../shared/api/client';

export type CurrencyDisplay = 'SHOW' | 'ASK' | 'HIDE';

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
