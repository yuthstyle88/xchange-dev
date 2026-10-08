export function homeOf(user: { role: string }): string {
  return user.role === 'FRONT' ? '/desk' : '/';
}
