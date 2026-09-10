import { Icon } from './icons';

export type MetricTone = 'info' | 'accent' | 'success' | 'danger' | 'warning' | 'meta';
export type MetricIconName = 'lanes' | 'merge' | 'clock' | 'bolt' | 'msg' | 'token' | 'shield';
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
      <Icon name={name} size={14} />
    </span>
  );
}
