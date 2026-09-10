import { cloneElement } from 'react';
import { sidebarIcons } from './sidebar-icons';

const navigation = {
  dashboard: sidebarIcons.dashboard,
  sessions: sidebarIcons.sessions,
  prs: sidebarIcons.prs,
  merge: sidebarIcons.prs,
  rulebook: sidebarIcons.rulebook,
  leaderboard: sidebarIcons.leaderboard,
  cloud: sidebarIcons.hub,
  moon: sidebarIcons.theme,
  gear: sidebarIcons.settings,
};
const drawings = {
  lanes: (
    <g fill="currentColor" stroke="none">
      <rect x="3" y="5" width="18" height="3" rx="1.5" />
      <rect x="3" y="10.5" width="12" height="3" rx="1.5" />
      <rect x="3" y="16" width="15" height="3" rx="1.5" />
    </g>
  ),
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </>
  ),
  bolt: <path d="M13 2L4 14h7l-1 8 9-12h-7z" />,
  msg: <path d="M4 5h16v11H9l-5 4z" />,
  token: (
    <>
      <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z" />
      <path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" />
    </>
  ),
  shield: <path d="M12 3l8 4v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z" />,
  share: (
    <>
      <path d="M12 3v12M8 7l4-4 4 4M5 13v6h14v-6" />
    </>
  ),
  scan: (
    <>
      <path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M4 12h16" />
    </>
  ),
  copy: (
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a1 1 0 0 1 1-1h10" />
    </>
  ),
  calendar: (
    <>
      <rect x="3" y="5" width="18" height="16" rx="2" />
      <path d="M3 10h18M8 3v4M16 3v4" />
    </>
  ),
};

export type IconName = keyof typeof navigation | keyof typeof drawings;
export interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
}

/** Shared design artwork; the containing control supplies its accessible name. */
export function Icon({ name, size = 14, className }: IconProps) {
  if (Object.hasOwn(navigation, name)) {
    return cloneElement(navigation[name as keyof typeof navigation], {
      width: size,
      height: size,
      className,
      focusable: 'false',
    });
  }
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin={name === 'share' ? undefined : 'round'}
      aria-hidden="true"
      focusable="false"
    >
      {drawings[name as keyof typeof drawings]}
    </svg>
  );
}
