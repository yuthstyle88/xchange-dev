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
  mustChangePassword?: boolean;
}

export interface AuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }>;
}

export interface PasswordApi {
  changePassword(p: { currentPassword: string; newPassword: string }): Promise<void>;
}

export function authApi(api: Pick<ApiClient, 'post'>): AuthApi & PasswordApi {
  return {
    session(): Promise<{ user: SessionUser | null }> {
      return api.post('/api/auth/session', {});
    },
    login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }> {
      return api.post('/api/auth/login', { identifier: p.identifier, password: p.password });
    },
    async changePassword(p: { currentPassword: string; newPassword: string }): Promise<void> {
      await api.post('/api/auth/password', { currentPassword: p.currentPassword, newPassword: p.newPassword });
    },
  };
}
