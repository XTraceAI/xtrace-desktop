import { useState } from 'react';
import {
  DataTable,
  TitleCell,
  NumCell,
  MonoCell,
  EmptyState,
  LoadingRows,
  FilterMenu,
  SparkBars,
  DayStrip,
  Legend,
  KindBadge,
  HostGlyph,
} from '../../kit';
import type { Column, SortOrder } from '../../kit/DataTable';
import { story } from '../story';
import { sample } from '../sample';
import { AutoActivate } from '../AutoActivate';

type Row = (typeof sample.rows)[number];
function TableExample({
  height = 40,
  mode = '',
}: {
  height?: 22 | 24 | 32 | 40 | 44;
  mode?: string;
}) {
  const [expanded, setExpanded] = useState<string[]>(mode === 'expanded' ? ['a'] : []);
  const [sort, setSort] = useState<SortOrder>({ key: 'title', direction: 'asc' });
  const [selected, setSelected] = useState('No example row selected');
  const columns: Column<Row>[] = [
    {
      key: 'title',
      header: 'Illustrative title',
      width: 'minmax(0, 1fr)',
      sortable: true,
      render: (row) =>
        height > 32 ? (
          <TitleCell title={row.title} subline={row.detail} />
        ) : (
          <MonoCell>{row.title}</MonoCell>
        ),
    },
    { key: 'kind', header: 'Kind', width: '100px', render: (row) => <KindBadge kind={row.kind} /> },
    { key: 'host', header: 'Host', width: '100px', render: (row) => <HostGlyph host={row.host} /> },
    {
      key: 'count',
      header: 'Example count',
      width: '120px',
      align: 'right',
      render: (row) => <NumCell value={row.count} reason="Illustrative missing count" />,
    },
  ];
  const rows = [...sample.rows].sort(
    (a, b) => a.title.localeCompare(b.title) * (sort.direction === 'asc' ? 1 : -1),
  );
  return (
    <div>
      <DataTable
        label="Illustrative rows"
        columns={columns}
        rows={mode === 'empty' ? [] : rows}
        getRowKey={(row) => row.id}
        rowHeight={height}
        loading={mode === 'loading'}
        sort={sort}
        onSort={setSort}
        onRowClick={(row) => setSelected(`${row.title} selected`)}
        renderExpanded={
          mode === 'expanded' ? (row) => <p>Illustrative details for {row.title}</p> : undefined
        }
        expandedKeys={expanded}
        onExpandedChange={setExpanded}
        hover={height < 32 ? 'track' : 'subtle'}
        rowOpacity={mode === 'muted' ? (row) => (row.count === null ? 0.5 : 1) : undefined}
        maxHeight={mode === 'sticky' ? 105 : undefined}
        stickyHeader={mode === 'sticky'}
      />
      <p className="gallery-caption">{selected}</p>
    </div>
  );
}
function FilterExample({ mode }: { mode: string }) {
  const [selected, setSelected] = useState<string[]>(
    mode === 'all' ? [] : ['claude', 'unknown-host'],
  );
  const filter = (
    <div className="gallery-stack">
      <FilterMenu
        label="Illustrative host filter"
        options={mode === 'empty' ? [] : sample.hosts}
        selected={selected}
        onChange={setSelected}
      />
    </div>
  );
  return ['open', 'empty'].includes(mode) ? (
    <AutoActivate selector='[aria-label="Illustrative host filter"]'>{filter}</AutoActivate>
  ) : (
    filter
  );
}
export const tableStories = [
  ...(
    [
      { id: 'prs-40', height: 40 },
      { id: 'rulebook-44-expanded', height: 44, mode: 'expanded' },
      { id: 'lanes-22', height: 22 },
      { id: 'env-24', height: 24 },
      { id: 'tray-32', height: 32 },
    ] as const
  ).map((row) =>
    story(
      `datatable/${row.id}`,
      ['DataTable', 'TitleCell', 'NumCell', 'MonoCell'],
      [1000, 260],
      () => <TableExample height={row.height} mode={'mode' in row ? row.mode : undefined} />,
    ),
  ),
  ...['empty', 'loading', 'muted', 'sticky'].map((mode) =>
    story(`datatable/${mode}`, ['DataTable'], [1000, 240], () => <TableExample mode={mode} />),
  ),
  story(
    'cells/title-ellipsis-and-numbers',
    ['TitleCell', 'NumCell', 'MonoCell'],
    [390, 160],
    () => (
      <div className="gallery-stack">
        <TitleCell
          title="Illustrative long title that should truncate inside the constrained cell"
          subline="A separate illustrative subline"
        />
        <div className="gallery-row">
          <NumCell value={0} />
          <NumCell value={null} />
          <MonoCell>Example</MonoCell>
          <MonoCell muted>Muted example</MonoCell>
        </div>
      </div>
    ),
  ),
  story('empty/message', ['EmptyState'], [460, 130], () => (
    <EmptyState message="No illustrative rows" />
  )),
  story('empty/action', ['EmptyState'], [460, 170], () => (
    <EmptyState
      message="No illustrative rows"
      action={{ label: 'Reset example', onClick: () => {} }}
    />
  )),
  story('loading/rows', ['LoadingRows'], [460, 220], () => <LoadingRows rows={4} />),
  ...['closed', 'open', 'all', 'empty'].map((mode) =>
    story(`filtermenu/${mode}`, ['FilterMenu'], [350, 290], () => <FilterExample mode={mode} />),
  ),
  ...(['hours', 'fires', 'share'] as const).map((mode) =>
    story(`sparkbars/${mode}`, ['SparkBars'], [440, 110], () => (
      <div className="gallery-stack">
        <SparkBars
          label={`Illustrative ${mode}`}
          days={sample.days}
          max={6}
          height={mode === 'share' ? 44 : 36}
          single={mode === 'fires'}
        />
      </div>
    )),
  ),
  story('sparkbars/unknown-and-zero', ['SparkBars'], [360, 110], () => (
    <div className="gallery-stack">
      <SparkBars
        days={[
          { label: 'Zero example', agent: 0, human: 0 },
          { label: 'Missing example', agent: null },
          { label: 'Invalid example', agent: -1 },
          { label: 'Large example', agent: 12 },
        ]}
        max={6}
      />
    </div>
  )),
  story('daystrip/thresholds-zero-unknown', ['DayStrip'], [380, 80], () => (
    <div className="gallery-stack">
      <DayStrip days={sample.days} thresholds={[2, 4]} />
    </div>
  )),
  ...[false, true].map((dots) =>
    story(`legend/${dots ? 'dots' : 'swatches'}`, ['Legend'], [440, 100], () => (
      <div className="gallery-stack">
        <Legend
          dots={dots}
          items={[
            { label: 'Illustrative A', value: 8, tone: 'accent' },
            { label: 'Illustrative B', value: 0, tone: 'meta' },
            { label: 'Unknown', value: '—', tone: 'warning' },
          ]}
        />
      </div>
    )),
  ),
];
