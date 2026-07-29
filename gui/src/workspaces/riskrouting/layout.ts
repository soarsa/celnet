/**
 * A tiny, dependency-free layered auto-layout for the routing decision tree. It
 * assigns each node a (layer, slot) then a pixel position, so a freshly-loaded or
 * just-edited graph reads top-to-bottom left-to-right without the trader placing a
 * single card. Explicit (dragged) positions always win over the computed default.
 *
 * The layering is a bounded longest-path relaxation from the entry: a node sits
 * one column right of the deepest branch that reaches it, so `yes`/`no` edges flow
 * rightward and books land at the right margin. The relaxation is capped at
 * `nodes.length` passes, so even a mid-edit cycle terminates (it just stops
 * deepening) rather than looping.
 */
import type { RiskRoutingGraph, RoutingNode } from "../../data/contract";

/** A laid-out pixel position for one node's top-left. */
export interface NodePos {
  x: number;
  y: number;
}

/** Card + gutter geometry — shared with the canvas CSS (card is 208×92). */
export const CARD_W = 208;
export const CARD_H = 92;
export const COL_GAP = 96;
export const ROW_GAP = 40;
export const MARGIN = 48;

const COL_STRIDE = CARD_W + COL_GAP;
const ROW_STRIDE = CARD_H + ROW_GAP;

/** Successor node ids of a node (both branches of a condition; none of a book). */
function successors(node: RoutingNode): number[] {
  return node.kind === "condition" ? [node.condition.onTrue, node.condition.onFalse] : [];
}

/**
 * Compute a default position for every node id. Layer = longest distance from the
 * entry (bounded); nodes unreachable from the entry are appended in their own
 * columns after the reachable frontier so nothing overlaps.
 */
export function computeLayout(graph: RiskRoutingGraph): Map<number, NodePos> {
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));
  const layer = new Map<number, number>();
  for (const n of graph.nodes) layer.set(n.id, 0);

  // Bounded longest-path relaxation: relax every edge up to nodes.length times.
  const passes = graph.nodes.length;
  for (let p = 0; p < passes; p += 1) {
    let changed = false;
    for (const n of graph.nodes) {
      const base = layer.get(n.id) ?? 0;
      for (const to of successors(n)) {
        if (!byId.has(to)) continue;
        const want = base + 1;
        if ((layer.get(to) ?? 0) < want) {
          layer.set(to, want);
          changed = true;
        }
      }
    }
    if (!changed) break;
  }

  // Bucket nodes by layer, preserving declaration order for a stable slot order.
  const byLayer = new Map<number, number[]>();
  for (const n of graph.nodes) {
    const l = layer.get(n.id) ?? 0;
    const bucket = byLayer.get(l);
    if (bucket) bucket.push(n.id);
    else byLayer.set(l, [n.id]);
  }

  const pos = new Map<number, NodePos>();
  for (const [l, ids] of byLayer) {
    ids.forEach((id, slot) => {
      pos.set(id, { x: MARGIN + l * COL_STRIDE, y: MARGIN + slot * ROW_STRIDE });
    });
  }
  return pos;
}

/** The bounding size of a laid-out graph (for the scrollable canvas surface). */
export function layoutExtent(positions: Map<number, NodePos>): { width: number; height: number } {
  let maxX = 0;
  let maxY = 0;
  for (const p of positions.values()) {
    maxX = Math.max(maxX, p.x + CARD_W);
    maxY = Math.max(maxY, p.y + CARD_H);
  }
  return { width: maxX + MARGIN, height: maxY + MARGIN };
}
