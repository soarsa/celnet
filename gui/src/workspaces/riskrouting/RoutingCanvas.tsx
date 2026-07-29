/**
 * The decision-tree canvas — the marquee surface. Node cards are absolutely
 * positioned on a pannable / zoomable plane; `yes`(on_true) / `no`(on_false) edges
 * are drawn as labelled SVG bezier paths beneath the cards. A layered auto-layout
 * (see {@link ./layout}) seeds positions; cards can be dragged to reposition.
 *
 * Interactions:
 *  • Drag a palette chip onto empty canvas → create a node at the drop point; drop
 *    a field chip onto a card → rebind that card's field.
 *  • Drag a card by its body → reposition (pointer events, transform-only motion).
 *  • Click a card's `yes`/`no` port → enter connect mode; click a target card →
 *    wire that branch to it. The traversed trace path lights up node-by-node.
 */
import { useCallback, useMemo, useRef, useState } from "react";

import type { RiskRoutingGraph, RoutingNode } from "../../data/contract";
import type { TraceResult } from "../../lib/routeTrace";
import { fieldSpec, opGlyph } from "../../lib/routeFields";
import { decodeDrag, type DragPayload } from "./FieldPalette";
import { CARD_H, CARD_W, layoutExtent, type NodePos } from "./layout";
import { valueLabel } from "./nodeLabel";
import styles from "./RiskRoutingWorkspace.module.css";

/** A pending branch wire: which node's which branch is being connected. */
export interface Connecting {
  nodeId: number;
  branch: "onTrue" | "onFalse";
}

interface RoutingCanvasProps {
  graph: RiskRoutingGraph;
  positions: Map<number, NodePos>;
  trace: TraceResult | null;
  selectedId: number | null;
  connecting: Connecting | null;
  zoom: number;
  readOnly: boolean;
  invalidNodes: ReadonlySet<number>;
  bookName: (id: string) => string;
  onSelect: (nodeId: number) => void;
  onMoveNode: (nodeId: number, pos: NodePos) => void;
  onDropPayload: (payload: DragPayload, at: NodePos) => void;
  onDropFieldOnNode: (nodeId: number, payload: DragPayload) => void;
  onPortClick: (nodeId: number, branch: "onTrue" | "onFalse") => void;
  onConnectTo: (nodeId: number) => void;
  /** Drag-to-connect: wire `nodeId`'s `branch` directly to `target` (the primary path). */
  onConnect: (nodeId: number, branch: "onTrue" | "onFalse", target: number) => void;
}

/** A live drag-from-port wire following the cursor (surface coordinates). */
interface WireDrag {
  from: NodePos;
  to: NodePos;
}

const YES_DY = CARD_H * 0.36;
const NO_DY = CARD_H * 0.72;

export function RoutingCanvas(props: RoutingCanvasProps): React.ReactElement {
  const { graph, positions, trace, selectedId, connecting, zoom, readOnly } = props;
  const surfaceRef = useRef<HTMLDivElement>(null);
  const dragState = useRef<{ id: number; dx: number; dy: number } | null>(null);
  // Drag-from-port wiring: `wireRef` holds the live drag (source + branch + whether
  // it has moved — a no-move press falls through to click-to-connect); `wire` is the
  // rendered live path following the cursor.
  const wireRef = useRef<{ nodeId: number; branch: "onTrue" | "onFalse"; moved: boolean } | null>(
    null,
  );
  const [wire, setWire] = useState<WireDrag | null>(null);

  const extent = useMemo(() => layoutExtent(positions), [positions]);

  // The set of directed edges the trace walked, as "from>to" keys, for highlight.
  const pathEdges = useMemo(() => {
    const s = new Set<string>();
    if (trace) for (let i = 0; i < trace.path.length - 1; i += 1) s.add(`${trace.path[i]}>${trace.path[i + 1]}`);
    return s;
  }, [trace]);
  const pathNodes = useMemo(() => new Set(trace?.path ?? []), [trace]);

  /** Convert a client point to surface coordinates (accounting for scroll + zoom). */
  const toSurface = useCallback(
    (clientX: number, clientY: number): NodePos => {
      const el = surfaceRef.current;
      if (!el) return { x: 0, y: 0 };
      const r = el.getBoundingClientRect();
      return { x: (clientX - r.left) / zoom, y: (clientY - r.top) / zoom };
    },
    [zoom],
  );

  const onCanvasDrop = useCallback(
    (e: React.DragEvent): void => {
      e.preventDefault();
      if (readOnly) return;
      const payload = decodeDrag(e.dataTransfer.getData("text/plain"));
      if (!payload) return;
      const at = toSurface(e.clientX, e.clientY);
      props.onDropPayload(payload, { x: Math.max(0, at.x - CARD_W / 2), y: Math.max(0, at.y - CARD_H / 2) });
    },
    [readOnly, toSurface, props],
  );

  // --- card pointer drag ---------------------------------------------------
  const onCardPointerDown = useCallback(
    (nodeId: number) =>
      (e: React.PointerEvent): void => {
        if (readOnly) return;
        // Ignore drags that begin on an interactive control (port/button).
        if ((e.target as HTMLElement).closest(`.${styles.port}`)) return;
        const pos = positions.get(nodeId);
        if (!pos) return;
        const p = toSurface(e.clientX, e.clientY);
        dragState.current = { id: nodeId, dx: p.x - pos.x, dy: p.y - pos.y };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      },
    [readOnly, positions, toSurface],
  );

  const onCardPointerMove = useCallback(
    (e: React.PointerEvent): void => {
      const st = dragState.current;
      if (!st) return;
      const p = toSurface(e.clientX, e.clientY);
      props.onMoveNode(st.id, { x: Math.max(0, p.x - st.dx), y: Math.max(0, p.y - st.dy) });
    },
    [toSurface, props],
  );

  const onCardPointerUp = useCallback((e: React.PointerEvent): void => {
    dragState.current = null;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* pointer already released */
    }
  }, []);

  // --- drag-from-port wiring ----------------------------------------------
  const onPortPointerDown = useCallback(
    (nodeId: number, branch: "onTrue" | "onFalse") =>
      (e: React.PointerEvent): void => {
        if (readOnly) return;
        // Don't let the card's body-drag start from a port press.
        e.stopPropagation();
        const src = positions.get(nodeId);
        if (!src) return;
        const from = anchorRight(src, branch === "onTrue" ? YES_DY : NO_DY);
        wireRef.current = { nodeId, branch, moved: false };
        setWire({ from, to: from });
      },
    [readOnly, positions],
  );

  const onSurfacePointerMove = useCallback(
    (e: React.PointerEvent): void => {
      if (!wireRef.current) return;
      wireRef.current.moved = true;
      const to = toSurface(e.clientX, e.clientY);
      setWire((prev) => (prev ? { from: prev.from, to } : prev));
    },
    [toSurface],
  );

  // Release over empty canvas cancels the pending wire.
  const cancelWire = useCallback((): void => {
    if (wireRef.current) {
      wireRef.current = null;
      setWire(null);
    }
  }, []);

  // Release over a node card drops the wire onto it — but only when the press
  // actually dragged (a no-move press clears the wire and falls through to the
  // click-to-connect fallback). Returns whether the drop wired an edge.
  const dropWireOnNode = useCallback(
    (targetId: number): boolean => {
      const w = wireRef.current;
      if (!w) return false;
      const consumed = w.moved;
      wireRef.current = null;
      setWire(null);
      if (consumed) props.onConnect(w.nodeId, w.branch, targetId);
      return consumed;
    },
    [props],
  );

  return (
    <div className={styles.canvasScroll}>
      <div
        ref={surfaceRef}
        className={styles.canvasSurface}
        style={{
          width: Math.max(extent.width, 600),
          height: Math.max(extent.height, 400),
          transform: `scale(${zoom})`,
        }}
        onDragOver={(e) => {
          if (!readOnly) e.preventDefault();
        }}
        onDrop={onCanvasDrop}
        onPointerMove={onSurfacePointerMove}
        onPointerUp={cancelWire}
        data-testid="routing-canvas"
      >
        <svg className={styles.edges} width={Math.max(extent.width, 600)} height={Math.max(extent.height, 400)} aria-hidden>
          {graph.nodes.map((n) =>
            n.kind === "condition" ? (
              <EdgePair
                key={`e-${n.id}`}
                node={n}
                positions={positions}
                onPath={pathEdges}
              />
            ) : null,
          )}
          {wire && (
            <path
              d={bezier(wire.from, wire.to)}
              className={`${styles.edge} ${styles.wireDrag}`}
              fill="none"
              pointerEvents="none"
            />
          )}
        </svg>

        {graph.nodes.map((n) => {
          const pos = positions.get(n.id);
          if (!pos) return null;
          return (
            <NodeCard
              key={n.id}
              node={n}
              pos={pos}
              isEntry={graph.entry === n.id}
              isSelected={selectedId === n.id}
              inPath={pathNodes.has(n.id)}
              isLanded={trace?.outcome === "book" && trace.landedBook !== null && n.kind === "book" && n.bookId === trace.landedBook && trace.path[trace.path.length - 1] === n.id}
              invalid={props.invalidNodes.has(n.id)}
              connecting={connecting}
              readOnly={readOnly}
              bookName={props.bookName}
              onPointerDown={onCardPointerDown(n.id)}
              onPointerMove={onCardPointerMove}
              onPointerUp={onCardPointerUp}
              onSelect={() => (connecting ? props.onConnectTo(n.id) : props.onSelect(n.id))}
              onPort={(branch) => props.onPortClick(n.id, branch)}
              onPortPointerDown={(branch) => onPortPointerDown(n.id, branch)}
              onWireDrop={() => dropWireOnNode(n.id)}
              onDropField={(payload) => props.onDropFieldOnNode(n.id, payload)}
            />
          );
        })}
      </div>
    </div>
  );
}

// --- edges -----------------------------------------------------------------

function anchorRight(pos: NodePos, dy: number): NodePos {
  return { x: pos.x + CARD_W, y: pos.y + dy };
}
function anchorLeft(pos: NodePos): NodePos {
  return { x: pos.x, y: pos.y + CARD_H / 2 };
}

function bezier(a: NodePos, b: NodePos): string {
  const dx = Math.max(40, Math.abs(b.x - a.x) * 0.5);
  return `M ${a.x} ${a.y} C ${a.x + dx} ${a.y}, ${b.x - dx} ${b.y}, ${b.x} ${b.y}`;
}

function EdgePair({
  node,
  positions,
  onPath,
}: {
  node: Extract<RoutingNode, { kind: "condition" }>;
  positions: Map<number, NodePos>;
  onPath: ReadonlySet<string>;
}): React.ReactElement | null {
  const src = positions.get(node.id);
  if (!src) return null;
  const edges: { branch: "onTrue" | "onFalse"; to: number; dy: number }[] = [
    { branch: "onTrue", to: node.condition.onTrue, dy: YES_DY },
    { branch: "onFalse", to: node.condition.onFalse, dy: NO_DY },
  ];
  return (
    <>
      {edges.map(({ branch, to, dy }) => {
        const dst = positions.get(to);
        const from = anchorRight(src, dy);
        const yes = branch === "onTrue";
        if (!dst || to === node.id) {
          // Unwired (self-referencing placeholder): a small dashed stub.
          return (
            <circle
              key={branch}
              cx={from.x + 14}
              cy={from.y}
              r={5}
              className={`${styles.edgeStub} ${yes ? styles.edgeYes : styles.edgeNo}`}
            />
          );
        }
        const target = anchorLeft(dst);
        const highlighted = onPath.has(`${node.id}>${to}`);
        const mid = { x: (from.x + target.x) / 2, y: (from.y + target.y) / 2 };
        return (
          <g key={branch}>
            <path
              d={bezier(from, target)}
              className={`${styles.edge} ${yes ? styles.edgeYes : styles.edgeNo} ${highlighted ? styles.edgeOnPath : ""}`}
              fill="none"
            />
            <text x={mid.x} y={mid.y - 4} className={`${styles.edgeLabel} ${yes ? styles.edgeYesLabel : styles.edgeNoLabel}`}>
              {yes ? "yes" : "no"}
            </text>
          </g>
        );
      })}
    </>
  );
}

// --- node card -------------------------------------------------------------

interface NodeCardProps {
  node: RoutingNode;
  pos: NodePos;
  isEntry: boolean;
  isSelected: boolean;
  inPath: boolean;
  isLanded: boolean;
  invalid: boolean;
  connecting: Connecting | null;
  readOnly: boolean;
  bookName: (id: string) => string;
  onPointerDown: (e: React.PointerEvent) => void;
  onPointerMove: (e: React.PointerEvent) => void;
  onPointerUp: (e: React.PointerEvent) => void;
  onSelect: () => void;
  onPort: (branch: "onTrue" | "onFalse") => void;
  onPortPointerDown: (branch: "onTrue" | "onFalse") => (e: React.PointerEvent) => void;
  onWireDrop: () => boolean;
  onDropField: (payload: DragPayload) => void;
}

function NodeCard(props: NodeCardProps): React.ReactElement {
  const { node, pos, connecting } = props;

  const handlePointerUp = (e: React.PointerEvent): void => {
    // A pending wire dropped on this card wins over card-drag release.
    if (props.onWireDrop()) e.stopPropagation();
    props.onPointerUp(e);
  };
  const isBook = node.kind === "book";
  const cls = [
    styles.node,
    isBook ? styles.nodeBook : styles.nodeCondition,
    props.isSelected ? styles.nodeSelected : "",
    props.inPath ? styles.nodeInPath : "",
    props.isLanded ? styles.nodeLanded : "",
    props.invalid ? styles.nodeInvalid : "",
    connecting ? styles.nodeConnectTarget : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={cls}
      style={{ left: pos.x, top: pos.y, width: CARD_W, height: CARD_H }}
      onPointerDown={props.onPointerDown}
      onPointerMove={props.onPointerMove}
      onPointerUp={handlePointerUp}
      onClick={props.onSelect}
      onDragOver={(e) => {
        if (!props.readOnly) e.preventDefault();
      }}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const p = decodeDrag(e.dataTransfer.getData("text/plain"));
        if (p) props.onDropField(p);
      }}
      role="button"
      tabIndex={0}
      aria-label={isBook ? `Book node ${node.id}` : `Condition node ${node.id}`}
      data-testid={`node-${node.id}`}
      data-inpath={props.inPath ? "true" : undefined}
    >
      {props.isEntry && <span className={styles.entryBadge}>entry</span>}
      {isBook ? (
        <div className={styles.bookInner}>
          <span className={styles.bookGlyph} aria-hidden>
            ❦
          </span>
          <span className={styles.bookName}>
            {node.bookId.length === 0 ? "(no book)" : props.bookName(node.bookId)}
          </span>
        </div>
      ) : (
        <>
          <div className={styles.nodeField}>{fieldSpec(node.condition.field).label}</div>
          <div className={styles.nodeExpr}>
            <span className={styles.nodeOp}>{opGlyph(node.condition.op)}</span>
            <span className={styles.nodeVal}>{valueLabel(node.condition.value)}</span>
          </div>
          {!props.readOnly && (
            <div className={styles.ports}>
              <button
                type="button"
                className={`${styles.port} ${styles.portYes} ${connecting?.branch === "onTrue" && connecting.nodeId === node.id ? styles.portActive : ""}`}
                onPointerDown={props.onPortPointerDown("onTrue")}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onPort("onTrue");
                }}
                title="Drag to wire the yes branch (or click)"
                aria-label="Connect yes branch"
                data-testid={`port-${node.id}-onTrue`}
              >
                yes
              </button>
              <button
                type="button"
                className={`${styles.port} ${styles.portNo} ${connecting?.branch === "onFalse" && connecting.nodeId === node.id ? styles.portActive : ""}`}
                onPointerDown={props.onPortPointerDown("onFalse")}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onPort("onFalse");
                }}
                title="Drag to wire the no branch (or click)"
                aria-label="Connect no branch"
                data-testid={`port-${node.id}-onFalse`}
              >
                no
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
