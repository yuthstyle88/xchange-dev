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

export interface RegisterParams {
  phone: string;
  password: string;
  email?: string;
}

export interface AuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }>;
  claim(p: { identifier: string; password: string; inviteCode: string }): Promise<{ user: SessionUser }>;
  register(p: RegisterParams): Promise<unknown>;
}

export function authApi(api: Pick<ApiClient, 'post'>): AuthApi {
  return {
    session(): Promise<{ user: SessionUser | null }> {
      return api.post('/api/auth/session', {});
    },
    login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }> {
      return api.post('/api/auth/login', { identifier: p.identifier, password: p.password });
    },
    claim(p: { identifier: string; password: string; inviteCode: string }): Promise<{ user: SessionUser }> {
      return api.post('/api/auth/invite/claim', {
        identifier: p.identifier,
        password: p.password,
        inviteCode: p.inviteCode,
      });
    },
    register(p: RegisterParams): Promise<unknown> {
      const trimmedEmail = p.email?.trim();
      const body: { phone: string; password: string; email?: string } = {
        phone: p.phone,
        password: p.password,
      };
      if (trimmedEmail) {
        body.email = trimmedEmail;
      }
      return api.post('/api/auth/register', body);
    },
  };
}
