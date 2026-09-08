export type MetricTone = 'info' | 'accent' | 'success' | 'danger' | 'warning' | 'meta';
const drawings = {
  lanes: (
    <>
      <path d="M3 4h12v3H3zm5 6h13v3H8zm-5 7h14v3H3z" fill="currentColor" stroke="none" />
    </>
  ),
  merge: (
    <>
      <circle cx="6" cy="4" r="2" />
      <circle cx="6" cy="20" r="2" />
      <circle cx="18" cy="4" r="2" />
      <path d="M6 6v12m12-12v3c0 5-12 3-12 8" />
    </>
  ),
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 6v6l4 2" />
    </>
  ),
  bolt: <path d="m14 2-11 12h8l-1 8 11-12h-8Z" />,
  msg: <path d="M3 3h18v13H9l-6 5Z" />,
  token: (
    <>
      <path d="m12 2 9 5v10l-9 5-9-5V7Z" />
      <path d="m3 7 9 5 9-5m-9 5v10" />
    </>
  ),
  shield: (
    <>
      <path d="m12 2 8 3v7c0 6-8 10-8 10S4 18 4 12V5Z" />
      <path d="m8 12 3 3 5-6" />
    </>
  ),
};
export type MetricIconName = keyof typeof drawings;
const tones: Record<MetricIconName, MetricTone> = {
  lanes: 'info',
  merge: 'accent',
  clock: 'success',
  bolt: 'danger',
  msg: 'warning',
  token: 'info',
  shield: 'success',
};
export function MetricIcon({ name, tone }: { name: MetricIconName; tone?: MetricTone }) {
  return (
    <span className="xt-metric-icon" data-tone={tone ?? tones[name]}>
      <svg
        aria-hidden="true"
        focusable="false"
        width={14}
        height={14}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        {drawings[name]}
      </svg>
    </span>
  );
}
