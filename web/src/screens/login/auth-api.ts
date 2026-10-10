import type { ApiClient } from '../../shared/api/client';

export interface SessionUserRole {
  role: string;
  branchId: string | null;
}

export interface SessionUser {
  id: string;
  name: string;
  organizationId: string;
  roles: SessionUserRole[];
}

export interface AuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }>;
}

export function authApi(api: Pick<ApiClient, 'post'>): AuthApi {
  return {
    session(): Promise<{ user: SessionUser | null }> {
      return api.post('/api/auth/session', {});
    },
    login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }> {
      return api.post('/api/auth/login', { identifier: p.identifier, password: p.password });
    },
  };
}
