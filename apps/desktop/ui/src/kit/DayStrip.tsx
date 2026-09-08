import '../styles/tables.css';
export interface StripDay {
  label: string;
  value: number | null;
}
export function DayStrip({
  days,
  thresholds,
  label = 'Fourteen-day activity',
}: {
  days: readonly StripDay[];
  thresholds: readonly [number, number];
  label?: string;
}) {
  if (days.length !== 14) throw new Error('DayStrip requires exactly fourteen labelled days');
  const [low, high] = thresholds;
  if (!Number.isFinite(low) || !Number.isFinite(high) || low <= 0 || high <= low)
    throw new Error('DayStrip thresholds must be positive and increasing');
  return (
    <div className="xt-day-strip" role="group" aria-label={label}>
      {days.map(({ label: day, value }, index) => {
        const measured = typeof value === 'number' && Number.isFinite(value) && value >= 0;
        const level = !measured || value === 0 ? 0 : value < low ? 1 : value < high ? 2 : 3;
        const text = `${day}: ${measured ? value : 'unmeasured'}`;
        return (
          <span
            key={index}
            role="img"
            title={text}
            aria-label={text}
            data-level={level}
            data-unknown={!measured || undefined}
          />
        );
      })}
    </div>
  );
}
