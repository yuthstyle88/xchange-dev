import type { ApiClient } from '../../../../shared/api/client';
import type { ReversalPostedDto } from '../../../../shared/api/generated/reversal_post';
import { command } from '../../../staff/parts/command';

export interface Poster {
  post(): Promise<string>;
}

export function createPoster(api: ApiClient, id: string): Poster {
  const cmd = command<Record<string, never>, ReversalPostedDto>(
    api,
    'POST',
    `/api/reversals/${id}/post`,
  );
  return {
    async post(): Promise<string> {
      const res = await cmd.send({});
      return res.documentNumber;
    },
  };
}
