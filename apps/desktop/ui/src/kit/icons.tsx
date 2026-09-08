const drawings = {
  dashboard: <path d="M3 10 12 3l9 7v11h-7v-7h-4v7H3Z" />,
  sessions: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l4 2" />
    </>
  ),
  prs: (
    <>
      <circle cx="6" cy="5" r="2" />
      <circle cx="6" cy="19" r="2" />
      <circle cx="18" cy="19" r="2" />
      <path d="M6 7v10M18 17V9a4 4 0 0 0-4-4h-2m3-3-3 3 3 3" />
    </>
  ),
  rulebook: (
    <>
      <path d="M4 3h12a4 4 0 0 1 4 4v14H8a4 4 0 0 1-4-4Zm0 13a4 4 0 0 1 4-4h12M9 7h6" />
    </>
  ),
  leaderboard: (
    <>
      <path d="M4 21V11h5v10M9 21V4h6v17M15 21v-7h5v7M2 21h20" />
    </>
  ),
  cloud: <path d="M7 18a5 5 0 1 1 1-10 6 6 0 0 1 11 2 4 4 0 0 1-1 8Z" />,
  moon: <path d="M21 13A9 9 0 0 1 11 3a9 9 0 1 0 10 10Z" />,
  gear: (
    <>
      <circle cx="12" cy="12" r="7" />
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3m0 14v3M2 12h3m14 0h3M5 5l2 2m10 10 2 2M5 19l2-2M17 7l2-2" />
    </>
  ),
  share: (
    <>
      <path d="M12 16V3m-4 4 4-4 4 4M5 13v7h14v-7" />
    </>
  ),
  scan: (
    <>
      <path d="M3 8V3h5m8 0h5v5m0 8v5h-5m-8 0H3v-5M3 12h18" />
    </>
  ),
  copy: (
    <>
      <rect x="8" y="8" width="13" height="13" rx="2" />
      <path d="M16 8V3H3v13h5" />
    </>
  ),
};

export type IconName = keyof typeof drawings;
export interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
}

/** Original simple line drawings; the containing control supplies its accessible name. */
export function Icon({ name, size = 14, className }: IconProps) {
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
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {drawings[name]}
    </svg>
  );
}
