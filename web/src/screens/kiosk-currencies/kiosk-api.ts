import type { ApiClient } from '../../shared/api/client';

export interface ConfigurePayload {
  branchId: string;
  code: string;
  buy: string;
  sell: string;
  enabled: boolean;
  previousUpdated: string;
  partnerName: string;
  partnerCost: string | null;
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
