/**
 * Stories — DataGrid (the virtualised, roving-tabindex data grid primitive).
 *
 * The IB-scale table workhorse: uniform-height row virtualisation + column
 * windowing (both axes windowed for very wide, very tall books), optional
 * collapsible group header rows, sortable header cells, and a WAI-ARIA APG grid
 * roving-tabindex keyboard model. It is data-shape-agnostic (generic over the row
 * datum `T`): callers supply `columns` (accessor per column) and pre-grouped
 * `groups`, and the grid renders + windows them.
 *
 * These stories drive it with a small synthetic positions dataset so the columns,
 * grouping, sorting, and empty state are all reviewable offline (no transport).
 */

import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { DataGrid } from "./DataGrid";
import { EMPTY_GROUP_MODEL, toggleGroup, type ColumnDef, type RowGroup } from "../lib/grid";

/** A synthetic position row — the demo datum the stories render. */
interface Position {
  readonly id: string;
  readonly pair: string;
  readonly tenor: string;
  readonly delta: number;
  readonly vega: number;
  readonly theta: number;
}

const COLUMNS: ReadonlyArray<ColumnDef<Position>> = [
  { key: "pair", header: "Pair", width: 96, align: "left", accessor: (r) => r.pair, sortKey: "pair" },
  { key: "tenor", header: "Tenor", width: 80, align: "left", accessor: (r) => r.tenor },
  { key: "delta", header: "Δ", unit: "%", width: 90, accessor: (r) => (r.delta * 100).toFixed(1), sortKey: "delta" },
  { key: "vega", header: "ν", unit: "k/1%", width: 100, accessor: (r) => r.vega.toFixed(1), sortKey: "vega" },
  { key: "theta", header: "Θ", unit: "k/day", width: 100, accessor: (r) => r.theta.toFixed(2) },
];

const POSITIONS: readonly Position[] = [
  { id: "p1", pair: "EUR/USD", tenor: "1M", delta: 0.31, vega: 42.1, theta: -3.2 },
  { id: "p2", pair: "EUR/USD", tenor: "3M", delta: -0.18, vega: 61.4, theta: -2.1 },
  { id: "p3", pair: "GBP/USD", tenor: "1M", delta: 0.52, vega: 18.7, theta: -1.4 },
  { id: "p4", pair: "GBP/USD", tenor: "1Y", delta: -0.44, vega: 88.3, theta: -0.9 },
  { id: "p5", pair: "USD/JPY", tenor: "2W", delta: 0.09, vega: 12.2, theta: -4.6 },
  { id: "p6", pair: "USD/JPY", tenor: "6M", delta: 0.27, vega: 55.0, theta: -1.8 },
];

/** All rows in one implicit (ungrouped) group — `groupable` off. */
const FLAT_GROUPS: ReadonlyArray<RowGroup<Position>> = [
  { key: "", label: "", rows: POSITIONS.map((p) => ({ key: p.id, datum: p })) },
];

/** Rows partitioned into one group per pair — for the collapsible-header variant. */
const PAIR_GROUPS: ReadonlyArray<RowGroup<Position>> = ["EUR/USD", "GBP/USD", "USD/JPY"].map(
  (pair) => ({
    key: pair,
    label: pair,
    rows: POSITIONS.filter((p) => p.pair === pair).map((p) => ({ key: p.id, datum: p })),
  }),
);

const meta = {
  title: "Components/DataGrid",
  component: DataGrid,
  tags: ["autodocs"],
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "The virtualised, roving-tabindex data-grid primitive: row + column " +
          "windowing for IB-scale books, optional collapsible group headers, sortable " +
          "header cells, and the WAI-ARIA APG grid keyboard model. Generic over the " +
          "row datum; callers pass column accessors and pre-grouped rows.",
      },
    },
  },
} satisfies Meta<typeof DataGrid>;

export default meta;

type Story = StoryObj<typeof DataGrid>;

/**
 * Ungrouped grid — a flat positions table. All rows sit in a single implicit group
 * with `groupable` off, so no header rows are drawn; the grid windows the rows and
 * columns and renders the right-aligned numeric cells with their unit sub-labels.
 */
export const Default: Story = {
  render: () => (
    <div style={{ height: 320 }}>
      <DataGrid<Position> label="Positions" columns={COLUMNS} groups={FLAT_GROUPS} />
    </div>
  ),
};

/**
 * Grouped + collapsible — rows partitioned one group per pair, `groupable` on. Each
 * group contributes a header row (with its member count); clicking a header toggles
 * its collapsed state through the controlled `groupModel` / `onToggleGroup` pair.
 * The virtualiser windows both header and data rows in the new arrangement.
 */
export const Grouped: Story = {
  name: "Grouped by pair (collapsible)",
  render: () => {
    function GroupedDemo(): React.ReactElement {
      const [model, setModel] = useState(EMPTY_GROUP_MODEL);
      return (
        <div style={{ height: 320 }}>
          <DataGrid<Position>
            label="Positions by pair"
            columns={COLUMNS}
            groups={PAIR_GROUPS}
            groupable
            groupModel={model}
            onToggleGroup={(key) => setModel((m) => toggleGroup(m, key))}
          />
        </div>
      );
    }
    return <GroupedDemo />;
  },
  parameters: {
    docs: {
      description: {
        story:
          "One group per currency pair with collapsible header rows. The group model " +
          "is controlled: clicking a header calls onToggleGroup, which folds the group " +
          "to just its header + member count. Group state is a client-side toggle.",
      },
    },
  },
};

/**
 * Empty state — no rows. The grid renders the caller-supplied `emptyState` node in
 * place of the row viewport (the honest "nothing to show" affordance), keeping the
 * header band so the column structure stays legible.
 */
export const Empty: Story = {
  render: () => (
    <div style={{ height: 200 }}>
      <DataGrid<Position>
        label="Positions"
        columns={COLUMNS}
        groups={[]}
        emptyState={
          <p style={{ color: "var(--text-tertiary)", fontSize: "var(--type-body)", margin: 0 }}>
            No positions in scope.
          </p>
        }
      />
    </div>
  ),
  parameters: {
    docs: {
      description: {
        story:
          "With no rows the grid shows the supplied emptyState node instead of an " +
          "empty viewport — the honest empty affordance, not a blank grid.",
      },
    },
  },
};
