import type { ApiClient } from '../../shared/api/client';

export interface AuthUser {
  id: string;
  username: string;
  name: string;
  role: string;
  branchId: string | null;
}

export interface AuthApi {
  status(): Promise<{ user: AuthUser | null }>;
  login(p: { username: string; password: string }): Promise<{ user: AuthUser; mustChange?: boolean }>;
}

interface StatusResponse {
  setupRequired?: boolean;
  user: AuthUser | null;
}

interface LoginResponse {
  user: AuthUser;
  mustChange?: boolean;
}

export function authApi(api: Pick<ApiClient, 'post'>): AuthApi {
  return {
    async status(): Promise<{ user: AuthUser | null }> {
      const res = await api.post<{ action: string; payload: Record<string, never> }, StatusResponse>(
        '/api/auth',
        { action: 'status', payload: {} },
      );
      return { user: res.user };
    },
    async login(p: { username: string; password: string }): Promise<{ user: AuthUser; mustChange?: boolean }> {
      const res = await api.post<
        { action: string; payload: { username: string; password: string } },
        LoginResponse
      >('/api/auth', {
        action: 'login',
        payload: { username: p.username, password: p.password },
      });
      return {
        user: res.user,
        ...(res.mustChange !== undefined ? { mustChange: res.mustChange } : {}),
      };
    },
  };
}
