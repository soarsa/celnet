/**
 * RiskRoutingWorkspace — the drag-and-drop node/flow decision-tree canvas where a
 * trader defines the firm's risk-routing rules and routes each fill to a desk
 * portfolio (docs/FI-RISK-ROUTING-REQUIREMENTS.md §6.1, §8.6). It composes the
 * field palette, the flow canvas, the typed node editor, and the live "trace a
 * sample fill" panel; it loads the graph + rosters, validates client-side (the
 * `RiskRoutingGraph::validate` mirror) and only enables Save when the graph is one
 * the server will accept. Admin-gated edit; read-only otherwise.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  DeskDesc,
  FixConnection,
  RiskBook,
  RiskRoutingGraph,
  RouteOp,
  RouteValue,
} from "../../data/contract";
import { blankFill, traceGraph, validateGraph, type SampleFill } from "../../lib/routeTrace";
import { FieldPalette, decodeDrag, type DragPayload } from "./FieldPalette";
import {
  addBook,
  addCondition,
  deleteNode,
  replaceField,
  setBookTarget,
  setBranch,
  setEntry,
  setOperator,
  setValue,
} from "./graphOps";
import { computeLayout, type NodePos } from "./layout";
import { NodeEditor } from "./NodeEditor";
import { RoutingCanvas, type Connecting } from "./RoutingCanvas";
import { TracePanel } from "./TracePanel";
import styles from "./RiskRoutingWorkspace.module.css";

type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

const EMPTY_GRAPH: RiskRoutingGraph = { entry: 0, nodes: [] };
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 1.5;

export function RiskRoutingWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // Routing rules are FI risk management — a risk manager holding the FI trader
  // capability may edit (not admin-only); book STRUCTURE stays admin (Risk Books pane).
  const canEdit = auth.can("quote_respond", "fixed_income");
  const readOnly = !canEdit;

  const [graph, setGraph] = useState<RiskRoutingGraph>(EMPTY_GRAPH);
  const [baseline, setBaseline] = useState<string>(JSON.stringify(EMPTY_GRAPH));
  const [books, setBooks] = useState<RiskBook[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [overrides, setOverrides] = useState<Map<number, NodePos>>(new Map());
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [connecting, setConnecting] = useState<Connecting | null>(null);
  const [zoom, setZoom] = useState(1);
  const [fill, setFill] = useState<SampleFill>(blankFill());
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  // --- load -----------------------------------------------------------------
  useEffect(() => {
    if (!signedIn) return;
    let cancelled = false;
    void (async () => {
      try {
        const [g, b] = await Promise.all([app.transport.getRiskRoutingGraph(), app.transport.listRiskBooks()]);
        if (cancelled) return;
        const graphVal = g ?? EMPTY_GRAPH;
        setGraph(graphVal);
        setBaseline(JSON.stringify(graphVal));
        setBooks(b);
        setLoadError(null);
      } catch (e: unknown) {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load the routing graph");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  // Rosters for the enum value dropdowns (desks + FIX counterparties) — loaded for an
  // editor (FI-capability holder); a read-only viewer needs no pickers.
  useEffect(() => {
    if (!canEdit) return;
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => !cancelled && setDesks(d))
      .catch(() => undefined);
    void app.transport
      .listFixConnections()
      .then((c) => !cancelled && setConnections(c))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport, canEdit]);

  // --- derived --------------------------------------------------------------
  const knownBookIds = useMemo(
    () => new Set(books.filter((b) => b.enabled).map((b) => b.id)),
    [books],
  );
  const bookName = useCallback(
    (id: string): string => books.find((b) => b.id === id)?.name ?? id,
    [books],
  );
  const trace = useMemo(() => (graph.nodes.length > 0 ? traceGraph(graph, fill) : null), [graph, fill]);
  const issues = useMemo(() => validateGraph(graph, knownBookIds), [graph, knownBookIds]);
  const invalidNodes = useMemo(() => {
    const s = new Set<number>();
    for (const i of issues) if (i.node !== null) s.add(i.node);
    return s;
  }, [issues]);
  const dirty = JSON.stringify(graph) !== baseline;

  const positions = useMemo(() => {
    const computed = computeLayout(graph);
    for (const [id, p] of overrides) if (computed.has(id)) computed.set(id, p);
    return computed;
  }, [graph, overrides]);

  const selectedNode = useMemo(
    () => graph.nodes.find((n) => n.id === selectedId) ?? null,
    [graph.nodes, selectedId],
  );

  // --- edit handlers --------------------------------------------------------
  const applyGraph = useCallback((next: RiskRoutingGraph): void => {
    setGraph(next);
    setSaveState({ kind: "idle" });
  }, []);

  const onDropPayload = useCallback(
    (payload: DragPayload, at: NodePos): void => {
      const result = payload.kind === "field" ? addCondition(graph, payload.field) : addBook(graph);
      applyGraph(result.graph);
      setOverrides((m) => new Map(m).set(result.id, at));
      setSelectedId(result.id);
    },
    [graph, applyGraph],
  );

  const onDropFieldOnNode = useCallback(
    (nodeId: number, payload: DragPayload): void => {
      if (payload.kind !== "field") return;
      applyGraph(replaceField(graph, nodeId, payload.field));
      setSelectedId(nodeId);
    },
    [graph, applyGraph],
  );

  const onMoveNode = useCallback((nodeId: number, pos: NodePos): void => {
    setOverrides((m) => new Map(m).set(nodeId, pos));
  }, []);

  const onPortClick = useCallback((nodeId: number, branch: "onTrue" | "onFalse"): void => {
    setConnecting((c) => (c && c.nodeId === nodeId && c.branch === branch ? null : { nodeId, branch }));
  }, []);

  const onConnectTo = useCallback(
    (target: number): void => {
      if (!connecting) return;
      applyGraph(setBranch(graph, connecting.nodeId, connecting.branch, target));
      setConnecting(null);
    },
    [connecting, graph, applyGraph],
  );

  const onDelete = useCallback(
    (nodeId: number): void => {
      applyGraph(deleteNode(graph, nodeId));
      setOverrides((m) => {
        const next = new Map(m);
        next.delete(nodeId);
        return next;
      });
      setSelectedId(null);
    },
    [graph, applyGraph],
  );

  const save = useCallback(async (): Promise<void> => {
    if (issues.length > 0) {
      setSaveState({ kind: "error", message: "Resolve the highlighted issues before saving." });
      return;
    }
    setSaveState({ kind: "saving" });
    try {
      const saved = await app.transport.updateRiskRoutingGraph(graph);
      setGraph(saved);
      setBaseline(JSON.stringify(saved));
      setSaveState({ kind: "ok", message: "Routing graph saved." });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save the routing graph" });
    }
  }, [issues.length, graph, app.transport]);

  const resetGraph = useCallback((): void => {
    const restored = JSON.parse(baseline) as RiskRoutingGraph;
    setGraph(restored);
    setOverrides(new Map());
    setSelectedId(null);
    setConnecting(null);
    setSaveState({ kind: "idle" });
  }, [baseline]);

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.centerEmpty}>Sign in to view the risk-routing graph.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>Risk Routing</h1>
          <p className={styles.note}>
            Compose the firm-wide decision tree that routes every fill's risk into a desk book. Drag
            a field onto the canvas to add a rule; wire its <span className={styles.yesInline}>yes</span> /{" "}
            <span className={styles.noInline}>no</span> branches to the next test or a book leaf.{" "}
            {readOnly ? "Read-only view." : "Admin edit."}
          </p>
        </div>
        <div className={styles.headActions}>
          <div className={styles.zoomer} role="group" aria-label="Zoom">
            <button type="button" className={styles.zoomBtn} onClick={() => setZoom((z) => Math.max(ZOOM_MIN, z - 0.1))} aria-label="Zoom out">
              −
            </button>
            <span className={styles.zoomVal}>{Math.round(zoom * 100)}%</span>
            <button type="button" className={styles.zoomBtn} onClick={() => setZoom((z) => Math.min(ZOOM_MAX, z + 0.1))} aria-label="Zoom in">
              +
            </button>
          </div>
          {!readOnly && (
            <>
              <button type="button" className={styles.ghostBtn} onClick={resetGraph} disabled={!dirty}>
                Reset
              </button>
              <button
                type="button"
                className={styles.saveBtn}
                onClick={() => void save()}
                disabled={saveState.kind === "saving" || issues.length > 0 || !dirty}
                data-testid="save-graph"
              >
                {saveState.kind === "saving" ? "Saving…" : "Save graph"}
              </button>
            </>
          )}
        </div>
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      <div className={styles.statusRow}>
        {issues.length === 0 ? (
          <span className={styles.statusOk} data-testid="validation-status">
            ✓ Valid — {graph.nodes.length} node{graph.nodes.length === 1 ? "" : "s"}
          </span>
        ) : (
          <span className={styles.statusBad} data-testid="validation-status">
            ⚠ {issues.length} issue{issues.length === 1 ? "" : "s"} to resolve before saving
          </span>
        )}
        {connecting && (
          <span className={styles.connectHint}>
            Click a target node to wire the {connecting.branch === "onTrue" ? "yes" : "no"} branch, or
            the same port to cancel.
          </span>
        )}
        {saveState.kind === "ok" && <span className={styles.statusOk}>{saveState.message}</span>}
        {saveState.kind === "error" && (
          <span className={styles.statusBad} role="alert">
            {saveState.message}
          </span>
        )}
      </div>

      <div className={styles.body}>
        <FieldPalette readOnly={readOnly} />

        <div className={styles.canvasCol}>
          {graph.nodes.length === 0 ? (
            // The empty state is ALSO a drop target — otherwise the very first node
            // could never be created (there is no RoutingCanvas surface to drop onto
            // until at least one node exists).
            <div
              className={styles.canvasEmpty}
              data-testid="routing-canvas-empty"
              onDragOver={(e) => {
                if (!readOnly) e.preventDefault();
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (readOnly) return;
                const payload = decodeDrag(e.dataTransfer.getData("text/plain"));
                if (payload) onDropPayload(payload, { x: 120, y: 100 });
              }}
            >
              <p>The routing graph is empty.</p>
              <p className={styles.canvasEmptyHint}>
                {readOnly
                  ? "No routing rules are defined yet."
                  : "Drag a field chip from the left onto this canvas to add your first rule, then drop a Book leaf for its destination."}
              </p>
            </div>
          ) : (
            <RoutingCanvas
              graph={graph}
              positions={positions}
              trace={trace}
              selectedId={selectedId}
              connecting={connecting}
              zoom={zoom}
              readOnly={readOnly}
              invalidNodes={invalidNodes}
              bookName={bookName}
              onSelect={setSelectedId}
              onMoveNode={onMoveNode}
              onDropPayload={onDropPayload}
              onDropFieldOnNode={onDropFieldOnNode}
              onPortClick={onPortClick}
              onConnectTo={onConnectTo}
            />
          )}
        </div>

        <div className={styles.side}>
          {selectedNode ? (
            <NodeEditor
              node={selectedNode}
              graph={graph}
              books={books}
              desks={desks}
              connections={connections}
              issues={issues.filter((i) => i.node === selectedNode.id)}
              readOnly={readOnly}
              isEntry={graph.entry === selectedNode.id}
              onSetOperator={(id, op: RouteOp) => applyGraph(setOperator(graph, id, op))}
              onSetValue={(id, v: RouteValue) => applyGraph(setValue(graph, id, v))}
              onSetBranch={(id, branch, target) => applyGraph(setBranch(graph, id, branch, target))}
              onSetBookTarget={(id, bookId) => applyGraph(setBookTarget(graph, id, bookId))}
              onSetEntry={(id) => applyGraph(setEntry(graph, id))}
              onDelete={onDelete}
            />
          ) : (
            <section className={styles.editor} aria-label="Node editor">
              <p className={styles.editorEmpty}>Select a node to edit it, or drag a field onto the canvas.</p>
            </section>
          )}

          <TracePanel fill={fill} trace={trace} books={books} onChange={setFill} />
        </div>
      </div>
    </div>
  );
}
