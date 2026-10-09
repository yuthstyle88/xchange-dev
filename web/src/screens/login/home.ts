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
