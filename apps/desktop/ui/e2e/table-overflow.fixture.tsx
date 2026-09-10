import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, useTheme } from '../src/theme/ThemeProvider';
import {
  DataTable,
  TitleCell,
  NumCell,
  MonoCell,
  type Column,
  type SortOrder,
} from '../src/kit/DataTable';
import { SectionCard } from '../src/kit/SectionCard';
import { FilterMenu } from '../src/kit/FilterMenu';
import { SparkBars } from '../src/kit/SparkBars';
import { DayStrip } from '../src/kit/DayStrip';
import { Legend } from '../src/kit/Legend';
import { EmptyState, LoadingRows } from '../src/kit/EmptyState';
import '../src/index.css';
import './table-overflow.css';
import { Button } from '../src/kit/Button';
import { KindBadge } from '../src/kit/Badge';
import { ProgressBar } from '../src/kit/ProgressBar';

type Sample = { id: string; title: string; tokens: number | null; state: string };
const sample: Sample[] = [
  { id: 'sample-1', title: 'Add session filters', tokens: 0, state: 'merged' },
  { id: 'sample-2', title: 'Handle unknown measurements', tokens: null, state: 'open' },
  { id: 'sample-3', title: 'Improve activity layout', tokens: 15000, state: 'excluded' },
];
const columns: Column<Sample>[] = [
  {
    key: 'title',
    header: 'Pull request',
    width: 'minmax(220px,1fr)',
    sortable: true,
    render: (row) => <TitleCell title={row.title} subline={row.id} />,
  },
  {
    key: 'tokens',
    header: 'Tokens',
    width: '100px',
    align: 'right',
    sortable: true,
    render: (row) => <NumCell value={row.tokens} reason="Usage absent" />,
  },
  {
    key: 'agent',
    header: 'Agent hours',
    width: '120px',
    align: 'right',
    render: () => <NumCell value={2.5} />,
  },
  {
    key: 'human',
    header: 'Messages',
    width: '100px',
    align: 'right',
    render: () => <NumCell value={12} />,
  },
  { key: 'type', header: 'Type', width: '100px', render: () => <MonoCell>feature</MonoCell> },
  { key: 'evidence', header: 'Evidence', width: '100px', render: () => <MonoCell>exact</MonoCell> },
  {
    key: 'state',
    header: 'State',
    width: '100px',
    render: (row) => <MonoCell muted={row.state === 'excluded'}>{row.state}</MonoCell>,
  },
  {
    key: 'action',
    header: 'Action',
    width: '90px',
    render: () => (
      <label>
        <input type="checkbox" /> Select row
      </label>
    ),
  },
];
function Fixture() {
  const { theme, toggle } = useTheme();
  const [ruleExpanded, setRuleExpanded] = useState<string[]>(['sample-2']);
  const [selected, setSelected] = useState(['claude', 'codex']);
  const [expanded, setExpanded] = useState<string[]>(['sample-1']);
  const [sort, setSort] = useState<SortOrder>({ key: 'tokens', direction: 'desc' });
  const [clicked, setClicked] = useState('none');
  const [activations, setActivations] = useState(0);
  const sorted = [...sample].sort((a, b) => {
    if (sort.key === 'tokens') {
      if (a.tokens === null) return b.tokens === null ? 0 : 1;
      if (b.tokens === null) return -1;
      return (a.tokens - b.tokens) * (sort.direction === 'asc' ? 1 : -1);
    }
    return a.title.localeCompare(b.title) * (sort.direction === 'asc' ? 1 : -1);
  });
  const days = Array.from({ length: 14 }, (_, index) => ({
    label: `Day ${index + 1}`,
    value: index === 3 ? null : index % 10,
  }));
  return (
    <main className="table-fixture">
      <div className="fixture-actions">
        <h1>Tables and activity</h1>
        <Button variant="outline" onClick={toggle}>
          Switch to {theme === 'dark' ? 'light' : 'dark'} theme
        </Button>
      </div>
      <p>Synthetic examples · Try sorting, row selection, expansion, and the host filter</p>
      <div className="fixture-actions">
        <FilterMenu
          selected={selected}
          onChange={setSelected}
          options={[
            { id: 'claude', label: 'Claude', count: 12 },
            { id: 'codex', label: 'Codex', count: 0 },
            { id: 'cursor', label: 'Cursor', count: null },
            { id: 'future', label: 'Future host', count: null },
          ]}
        />
        <Button variant="outline">Outside control</Button>
        <output aria-label="Selected hosts">{selected.join(', ') || 'all'}</output>
      </div>
      <SectionCard title="Pull requests" meta="Whole linked-session values may overlap" padding={0}>
        <DataTable
          label="Pull requests"
          columns={columns}
          rows={sorted}
          getRowKey={(row) => row.id}
          minWidth={1180}
          rowHeight={40}
          sort={sort}
          onSort={setSort}
          onRowClick={(row) => {
            setClicked(row.id);
            setActivations((count) => count + 1);
          }}
          getRowActionLabel={(row) => `Open ${row.title}`}
          rowOpacity={(row) => (row.state === 'excluded' ? 0.45 : 1)}
          expandedKeys={expanded}
          onExpandedChange={setExpanded}
          renderExpanded={(row) => <span>Synthetic details for {row.id}</span>}
          stickyHeader
          maxHeight={160}
        />
      </SectionCard>
      <div className="fixture-actions">
        <output aria-label="Clicked row">{clicked}</output>
        <output aria-label="Row activations">{activations}</output>
      </div>
      <div data-testid="flexible-table" style={{ width: 600, maxWidth: '100%' }}>
        <DataTable
          label="Flexible title"
          columns={columns
            .slice(0, 2)
            .map((column) =>
              column.key === 'title' ? { ...column, width: 'minmax(0,1fr)' } : column,
            )}
          rows={[{ ...sample[0], title: 'Long synthetic title '.repeat(30) }]}
          getRowKey={(row) => row.id}
        />
      </div>
      <SectionCard title="Rules" padding={0}>
        <DataTable
          label="Rules"
          columns={[
            {
              key: 'directive',
              header: 'Directive · source',
              width: 'minmax(220px,1fr)',
              render: (row) => <TitleCell title={row.title} subline="Synthetic rule" />,
            },
            {
              key: 'class',
              header: 'Class',
              width: '72px',
              render: () => <KindBadge kind="hook" />,
            },
            {
              key: 'mechanism',
              header: 'Mechanism',
              width: '150px',
              render: () => <MonoCell>Local check</MonoCell>,
            },
            {
              key: 'backtest',
              header: 'Backtest',
              width: '150px',
              render: () => (
                <ProgressBar
                  label="Rule backtest"
                  segments={[
                    { label: 'Followed', value: 80, tone: 'success' },
                    { label: 'Ignored', value: 20, tone: 'danger' },
                  ]}
                />
              ),
            },
            {
              key: 'mode',
              header: 'Mode',
              width: '90px',
              render: () => <MonoCell>advise</MonoCell>,
            },
            {
              key: 'step',
              header: 'Step',
              width: '110px',
              align: 'right',
              render: (row) => (
                <Button variant="outline" height={28} onClick={() => setClicked(row.id)}>
                  Review
                </Button>
              ),
            },
          ]}
          rows={[
            { ...sample[0], title: 'Check changes before publishing' },
            { ...sample[1], title: 'Keep missing observations explicit' },
          ]}
          getRowKey={(row) => row.id}
          rowHeight={44}
          minWidth={1000}
          expandedKeys={ruleExpanded}
          onExpandedChange={setRuleExpanded}
          renderExpanded={() => <span>Expanded synthetic rule detail</span>}
        />
      </SectionCard>
      <div className="fixture-pair">
        <SectionCard title="Compact rows" padding={0}>
          {([22, 24, 32] as const).map((height) => (
            <DataTable
              key={height}
              label={`Rows ${height}`}
              columns={[
                {
                  key: 'name',
                  header: 'Session',
                  width: 'minmax(0,1fr)',
                  render: (r) => <MonoCell>{r.id}</MonoCell>,
                },
                {
                  key: 'value',
                  header: 'Tokens',
                  width: '80px',
                  align: 'right',
                  render: (r) => <NumCell value={r.tokens} />,
                },
              ]}
              rows={sample.slice(0, 1)}
              getRowKey={(row) => row.id}
              rowHeight={height}
            />
          ))}
        </SectionCard>
        <SectionCard title="Activity">
          <SparkBars
            days={days.map((day) => ({
              label: day.label,
              agent: day.value,
              human: day.value === null ? null : day.value / 2,
            }))}
            max={9}
          />
          <Legend
            items={[
              { label: 'Agent', tone: 'accent', value: 45 },
              { label: 'Human', tone: 'warning', value: 22.5 },
            ]}
          />
          <DayStrip days={days} thresholds={[3, 8]} />
          <SparkBars
            days={days.map((day) => ({ label: day.label, agent: day.value }))}
            max={9}
            height={44}
            single
          />
        </SectionCard>
      </div>
      <div className="fixture-pair">
        <SectionCard title="Empty">
          <EmptyState
            message="No sessions in this window"
            action={{ label: 'Detect again', onClick: () => setClicked('detect') }}
          />
        </SectionCard>
        <SectionCard title="Loading">
          <LoadingRows rows={3} />
        </SectionCard>
      </div>
    </main>
  );
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <Fixture />
    </ThemeProvider>
  </StrictMode>,
);
