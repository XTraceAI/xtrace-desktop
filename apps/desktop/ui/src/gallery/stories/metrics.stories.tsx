import {
  StatTile,
  SectionCard,
  MetricCell,
  Unmeasured,
  RuleChip,
  RulePopover,
  MetricIcon,
  Button,
} from '../../kit';
import type { MetricIconName, MetricTone } from '../../kit/metric-icons';
import type { RuleId } from '../../kit/rules';
import { story } from '../story';
import { tokens, percent, hours } from '../../kit/format';
import { AutoActivate } from '../AutoActivate';

const metrics: {
  icon: MetricIconName;
  ruleId: RuleId;
  label: string;
  value: number;
  unit?: string;
}[] = [
  { icon: 'lanes', ruleId: 'M-06', label: 'Example concurrency', value: 2.4 },
  { icon: 'merge', ruleId: 'M-12', label: 'Example messages per PR', value: 3 },
  { icon: 'clock', ruleId: 'M-05', label: 'Example agent hours', value: 1.8, unit: 'h' },
  { icon: 'bolt', ruleId: 'M-08', label: 'Example agent/human ratio', value: 1.6 },
  { icon: 'msg', ruleId: 'M-02', label: 'Example human messages', value: 9 },
  { icon: 'token', ruleId: 'M-04', label: 'Example tokens', value: 6400 },
  { icon: 'shield', ruleId: 'M-16', label: 'Example sessions per day', value: 3 },
];
export const metricStories = [
  ...metrics.map((props) =>
    story(`stattile/${props.icon}`, ['StatTile'], [300, 52], () => (
      <StatTile {...props} delta={0.04} />
    )),
  ),
  story('stattile/unmeasured', ['StatTile'], [300, 52], () => (
    <StatTile {...metrics[5]} value={null} reason="Illustrative missing observation" delta={0.04} />
  )),
  story('stattile/delta-bad', ['StatTile'], [300, 52], () => (
    <StatTile {...metrics[1]} delta={-0.08} deltaTone="bad" />
  )),
  story('stattile/zero-and-long-aside', ['StatTile'], [300, 52], () => (
    <StatTile
      {...metrics[0]}
      value={0}
      aside="Illustrative long context that intentionally truncates"
    />
  )),
  story('metric-cell/known-zero-unknown-formats', ['MetricCell', 'Unmeasured'], [500, 160], () => (
    <div className="gallery-grid">
      <MetricCell value={0} />
      <MetricCell value={42} />
      <MetricCell value={6400} format={tokens} />
      <MetricCell value={0.75} format={percent} />
      <MetricCell value={1.25} format={hours} />
      <MetricCell value={null} reason="Illustrative missing observation" />
      <MetricCell value="Example label" />
      <Unmeasured reason="No illustrative measurement supplied" />
    </div>
  )),
  story('metric-cell/sizes-and-alignment', ['MetricCell'], [560, 150], () => (
    <div className="gallery-grid">
      {([10.5, 11, 18, 24, 30] as const).map((size) => (
        <MetricCell key={size} value={42} size={size} align={size === 30 ? 'right' : 'left'} />
      ))}
    </div>
  )),
  story('metric-icon/tones', ['MetricIcon'], [400, 240], () => (
    <div className="gallery-grid">
      {metrics.map(({ icon }) => (
        <span key={icon} className="gallery-row">
          <MetricIcon name={icon} />
          {icon}
        </span>
      ))}
      {(['info', 'accent', 'success', 'danger', 'warning', 'meta'] satisfies MetricTone[]).map(
        (tone) => (
          <span key={tone} className="gallery-row">
            <MetricIcon name="token" tone={tone} />
            {tone}
          </span>
        ),
      )}
    </div>
  )),
  ...(['default', '36-header', 'footer', 'slots'] as const).map((mode) =>
    story(`sectioncard/${mode}`, ['SectionCard'], [600, 190], () => (
      <SectionCard
        title="Illustrative section"
        headerHeight={mode === '36-header' ? 36 : 40}
        ruleId={mode === 'slots' ? 'M-06' : undefined}
        meta={mode === 'slots' ? 'Example scope' : undefined}
        right={mode === 'slots' ? <Button>Example action</Button> : undefined}
        footer={mode === 'footer' ? 'Illustrative footer' : undefined}
      >
        <p className="gallery-caption">Component content supplied by the gallery.</p>
      </SectionCard>
    )),
  ),
  story('rulechip/sizes', ['RuleChip'], [260, 100], () => (
    <div className="gallery-row gallery-stack">
      <RuleChip ruleId="M-06" />
      <RuleChip ruleId="C-08" size="sm" />
    </div>
  )),
  story('rulepopover/open-context', ['RulePopover'], [560, 240], () => (
    <AutoActivate selector="button" hover>
      <div className="gallery-stack">
        <RulePopover ruleId="M-06" context="Illustrative context only">
          <Button>Example definition</Button>
        </RulePopover>
      </div>
    </AutoActivate>
  )),
];
