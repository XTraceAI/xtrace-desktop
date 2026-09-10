import { useEffect, useRef, useState } from 'react';
import {
  KindBadge,
  EvidenceDot,
  StatePill,
  Toggle,
  Segmented,
  Button,
  Icon,
  ProgressBar,
  Search,
} from '../../kit';
import type { ControlTone } from '../../kit/control-tone';
import { story } from '../story';
const tones: ControlTone[] = ['success', 'warning', 'danger', 'info', 'accent', 'meta'];
function ToggleExample({ initial, disabled = false }: { initial: boolean; disabled?: boolean }) {
  const [checked, setChecked] = useState(initial);
  return (
    <div className="gallery-stack">
      <Toggle
        label="Illustrative capture"
        checked={checked}
        onChange={setChecked}
        disabled={disabled}
        trailing="Example scope"
      />
      <p className="gallery-caption">{checked ? 'Enabled' : 'Disabled'} sample state</p>
    </div>
  );
}
function SegmentedExample({ mode }: { mode: string }) {
  const [value, setValue] = useState(mode === 'range' ? '14d' : 'gate');
  const options =
    mode === 'range'
      ? [
          { value: '7d', label: '7d' },
          { value: '14d', label: '14d' },
          { value: '30d', label: '30d' },
        ]
      : [
          { value: 'advise', label: 'Advise' },
          { value: 'gate', label: 'Gate' },
          { value: 'other', label: 'Unavailable', disabled: true },
        ];
  return (
    <div className="gallery-stack">
      <Segmented
        label="Example selection"
        value={value}
        options={options}
        onChange={setValue}
        disabled={mode === 'disabled'}
      />
      <p className="gallery-caption">Selected: {value}</p>
    </div>
  );
}
function SearchExample({ mode }: { mode: string }) {
  const [value, setValue] = useState(mode === 'filled' ? 'Illustrative query' : '');
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (mode !== 'shortcut') return;
    const focus = (event: KeyboardEvent) => {
      if (event.metaKey && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        ref.current?.focus();
      }
    };
    window.addEventListener('keydown', focus);
    return () => window.removeEventListener('keydown', focus);
  }, [mode]);
  return (
    <div className="gallery-stack">
      <Search
        ref={ref}
        label="Illustrative search"
        placeholder="Search examples"
        value={value}
        onValueChange={setValue}
        disabled={mode === 'disabled'}
        shortcut={mode === 'shortcut' ? '⌘ K' : undefined}
      />
      <p className="gallery-caption">
        {mode === 'shortcut' ? 'Focus this frame to use its shortcut.' : 'Local input state only.'}
      </p>
    </div>
  );
}
export const controlStories = [
  story('badges/kind-all', ['KindBadge'], [700, 90], () => (
    <div className="gallery-row gallery-stack">
      {(['skill', 'agent', 'mcp', 'hook', 'plugin', 'cmd', 'feature', 'bug', 'chore'] as const).map(
        (kind) => (
          <KindBadge key={kind} kind={kind} />
        ),
      )}
    </div>
  )),
  story('badges/evidence-all', ['EvidenceDot'], [400, 90], () => (
    <div className="gallery-row gallery-stack">
      {(['exact', 'sha', 'inferred'] as const).map((evidence) => (
        <span key={evidence} className="gallery-row">
          <EvidenceDot evidence={evidence} />
          {evidence}
        </span>
      ))}
    </div>
  )),
  story('badges/statepill-tones', ['StatePill'], [600, 90], () => (
    <div className="gallery-row gallery-stack">
      {tones.map((tone) => (
        <StatePill key={tone} tone={tone}>
          {tone}
        </StatePill>
      ))}
    </div>
  )),
  story('badges/statepill-size-outline-live', ['StatePill'], [600, 130], () => (
    <div className="gallery-grid">
      {([20, 24, 26] as const).map((height) => (
        <StatePill key={height} height={height} outlined tone="info">
          {height}px
        </StatePill>
      ))}
      <StatePill live tone="success">
        Illustrative live
      </StatePill>
    </div>
  )),
  ...[false, true].flatMap((initial) =>
    [false, true].map((disabled) =>
      story(
        `toggle/${initial ? 'on' : 'off'}${disabled ? '-disabled' : ''}`,
        ['Toggle'],
        [330, 100],
        () => <ToggleExample initial={initial} disabled={disabled} />,
      ),
    ),
  ),
  ...['range', 'mode-gate', 'disabled'].map((mode) =>
    story(`segmented/${mode}`, ['Segmented'], [360, 100], () => <SegmentedExample mode={mode} />),
  ),
  story('button/variants-heights-disabled-icons', ['Button'], [680, 230], () => (
    <div className="gallery-grid">
      {(['primary', 'accent', 'outline', 'ghost'] as const).flatMap((variant) =>
        ([28, 30, 34, 38] as const).map((height) => (
          <Button
            key={`${variant}-${height}`}
            variant={variant}
            height={height}
            disabled={height === 34}
            icon={height === 38 ? <Icon name="copy" /> : undefined}
          >
            {variant} {height}
          </Button>
        )),
      )}
    </div>
  )),
  ...([3, 4, 6] as const).map((height) =>
    story(`progress/${height}`, ['ProgressBar'], [430, 90], () => (
      <div className="gallery-stack">
        <ProgressBar label="Illustrative completion" height={height} value={65} />
      </div>
    )),
  ),
  story('progress/zero-unknown-clamped', ['ProgressBar'], [430, 150], () => (
    <div className="gallery-stack">
      {[0, null, 150, -2].map((value, index) => (
        <ProgressBar key={index} label={`Illustrative state ${index}`} value={value} />
      ))}
    </div>
  )),
  story('progress/segmented', ['ProgressBar'], [430, 90], () => (
    <div className="gallery-stack">
      <ProgressBar
        label="Illustrative composition"
        segments={[
          { label: 'Example A', value: 45, tone: 'success' },
          { label: 'Example B', value: 25, tone: 'warning' },
        ]}
      />
    </div>
  )),
  ...['empty', 'filled', 'disabled', 'shortcut'].map((mode) =>
    story(`search/${mode}`, ['Search'], [410, 110], () => <SearchExample mode={mode} />),
  ),
];
