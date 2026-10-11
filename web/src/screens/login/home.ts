export interface SessionUserRole {
  role: string;
  branchId: string | null;
}

export function homeOf(user: { roles: ReadonlyArray<SessionUserRole> }): string {
  const hasOrgExecutive = user.roles.some((r) => r.role === 'EXECUTIVE' && r.branchId === null);
  const hasOrgAdmin = user.roles.some((r) => r.role === 'ADMIN' && r.branchId === null);
  if (hasOrgExecutive && hasOrgAdmin) {
    return '/';
  }
  if (user.roles.some((r) => r.role === 'TELLER')) {
    return '/desk';
  }
  return '/';
}

export function safeNext(raw: string | null | undefined): string | null {
  if (raw == null || raw === '') return null;
  let v: string;
  try {
    v = decodeURIComponent(raw);
  } catch {
    return null;
  }
  if (!v || /[\x00-\x1f\x7f\\]/.test(v)) return null;
  if (!v.startsWith('/') || v.startsWith('//') || v.startsWith('/\\')) return null;
  let u: URL;
  try {
    const origin = typeof window !== 'undefined' && window.location?.origin ? window.location.origin : 'http://localhost';
    u = new URL(v, origin);
    if (u.origin !== origin) return null;
  } catch {
    return null;
  }
  if (u.pathname + u.search + u.hash !== v) return null;
  if (
    u.pathname === '/login' ||
    u.pathname.startsWith('/login/') ||
    u.pathname === '/api' ||
    u.pathname.startsWith('/api/')
  ) {
    return null;
  }
  return v;
}
