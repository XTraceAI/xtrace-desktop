import type { SidebarKey } from '../kit/Sidebar';
export const pages: { path: string; title: string; crumb: string; active?: SidebarKey }[] = [
  { path: '/first-launch', title: 'Welcome to XTrace', crumb: 'first-launch' },
  { path: '/dashboard', title: 'Dashboard', crumb: 'dashboard', active: 'dashboard' },
  { path: '/sessions', title: 'Sessions', crumb: 'sessions', active: 'sessions' },
  { path: '/prs', title: 'Pull requests', crumb: 'pull-requests', active: 'prs' },
  { path: '/rulebook', title: 'Rulebook', crumb: 'rulebook', active: 'rulebook' },
  { path: '/rulebook/fires', title: 'Rule fires', crumb: 'rulebook', active: 'rulebook' },
  { path: '/rulebook/:ruleId', title: 'Rule detail', crumb: 'rulebook', active: 'rulebook' },
  { path: '/settings', title: 'Settings', crumb: 'settings' },
  { path: '/leaderboard', title: 'Leaderboard', crumb: 'leaderboard', active: 'leaderboard' },
];
