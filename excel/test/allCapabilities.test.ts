import { describe, expect, it } from "vitest";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  formatMarginSpill,
  formatPreTradeMarginSpill,
  formatAlgoOrderSpill,
  formatAlgoOrdersListSpill,
  formatClusterTopologySpill,
  formatUpgradeStatusSpill,
  formatCdmSpill,
  formatAttestationSpill,
  formatLicenseCapabilitiesSpill,
} from "../src/functions/shaping";
import type {
  MarginCalculationResponse,
  PreTradeMarginResponse,
  AlgoOrderResponse,
  ListAlgoOrdersResponse,
  ClusterTopologyResponse,
  UpgradeStatusResponse,
  ExportCdmResponse,
  AttestationResponse,
  LicenseCapabilityResponse,
} from "../src/contract/contract";

class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

describe("Excel SDK All Capabilities & Formatting", () => {
  it("formats Margin spill matrix correctly", () => {
    const resp: MarginCalculationResponse = {
      portfolioId: "PORTFOLIO-GLOBAL-1",
      totalInitialMargin: 4500000.0,
      expectedShortfall: 3800000.0,
      valueAtRisk: 3100000.0,
      stressComponent: 700000.0,
      currency: "USD",
      calculatedEpochNanos: 1772841600000000000n,
    };
    const spill = formatMarginSpill(resp);
    expect(spill.length).toBe(7);
    expect(spill[0]).toEqual(["Metric", "Value", "Currency"]);
    expect(spill[1]).toEqual(["Total Initial Margin", 4500000.0, "USD"]);
    expect(spill[2]).toEqual(["Expected Shortfall (ES)", 3800000.0, "USD"]);
  });

  it("formats PreTradeMargin spill matrix correctly", () => {
    const resp: PreTradeMarginResponse = {
      portfolioId: "PORTFOLIO-1",
      outcome: "APPROVED",
      initialMarginBefore: 1000000.0,
      initialMarginAfter: 1250000.0,
      deltaMargin: 250000.0,
      collateralHeadroom: 750000.0,
      reason: "Within unencumbered collateral limits",
    };
    const spill = formatPreTradeMarginSpill(resp);
    expect(spill.length).toBe(8);
    expect(spill[0]).toEqual(["Field", "Value"]);
    expect(spill[1]).toEqual(["Pre-Trade Outcome", "APPROVED"]);
    expect(spill[4]).toEqual(["Delta Margin Impact", 250000.0]);
  });

  it("formats Algo and AlgoOrders spill matrices correctly", () => {
    const algoResp: AlgoOrderResponse = {
      parentOrderId: "ALGO-TWAP-99",
      clientOrderId: "CLIENT-REF-1",
      symbol: "EURUSD",
      totalQuantity: 10000000,
      executedQuantity: 4000000,
      arrivalPrice: 1.085,
      avgExecPrice: 1.0851,
      isBuy: true,
      status: "ACTIVE",
      implementationShortfallBps: 0.92,
      slices: [
        {
          sliceIndex: 1,
          scheduledOffsetSeconds: 0,
          targetQuantity: 2000000,
          filledQuantity: 2000000,
          avgFillPrice: 1.08505,
          status: "FILLED",
        },
        {
          sliceIndex: 2,
          scheduledOffsetSeconds: 60,
          targetQuantity: 2000000,
          filledQuantity: 2000000,
          avgFillPrice: 1.08515,
          status: "FILLED",
        },
      ],
      createdEpochNanos: 1772841600000000000n,
    };
    const spill = formatAlgoOrderSpill(algoResp);
    expect(spill[0]![0]).toBe("Order ID");
    expect(spill[1]![0]).toBe("ALGO-TWAP-99");
    expect(spill[1]![1]).toBe("EURUSD");
    expect(spill[1]![2]).toBe("BUY");

    const listResp: ListAlgoOrdersResponse = {
      orders: [algoResp],
    };
    const listSpill = formatAlgoOrdersListSpill(listResp);
    expect(listSpill.length).toBe(2);
    expect(listSpill[1]![0]).toBe("ALGO-TWAP-99");
  });

  it("formats ClusterTopology spill correctly", () => {
    const clusterResp: ClusterTopologyResponse = {
      clusterId: "celnet-primary-prod",
      leaderId: "node-1-prod",
      activeGeneration: 42n,
      members: [
        {
          nodeId: "node-1-prod",
          endpoint: "tcp://10.0.1.1:9090",
          status: "ACTIVE",
          activeInFlightTrades: 1250n,
          joinedEpochNanos: 1772841600000000000n,
        },
        {
          nodeId: "node-2-prod",
          endpoint: "tcp://10.0.1.2:9090",
          status: "ACTIVE",
          activeInFlightTrades: 980n,
          joinedEpochNanos: 1772841600000000000n,
        },
      ],
      jointConsensusActive: false,
    };
    const spill = formatClusterTopologySpill(clusterResp);
    expect(spill[0]).toEqual(["Cluster ID", "celnet-primary-prod", "", ""]);
    expect(spill[1]).toEqual(["Leader ID", "node-1-prod", "", ""]);
    expect(spill[5]).toEqual(["node-1-prod", "tcp://10.0.1.1:9090", "ACTIVE", 1250]);
  });

  it("formats UpgradeStatus spill correctly", () => {
    const upgradeResp: UpgradeStatusResponse = {
      activeGeneration: 43n,
      currentVersion: "2026.9.1",
      shadowVersion: "2026.9.2",
      twinComparisonPassed: true,
      maxUlpDivergence: 0n,
      evaluatedTradesCount: 500000n,
      cutoverStatus: "READY_FOR_CUTOVER",
    };
    const spill = formatUpgradeStatusSpill(upgradeResp);
    expect(spill[3]).toEqual(["Shadow Twin Version", "2026.9.2"]);
    expect(spill[4]).toEqual(["Twin Bit-Exact Passed", "PASS (0 ULP)"]);
    expect(spill[5]).toEqual(["Max ULP Divergence", 0]);
  });

  it("formats CDM, Attestation, and License spills correctly", () => {
    const cdmResp: ExportCdmResponse = {
      executionId: 987654n,
      uti: "UTI-2026-US987654-001",
      cdmEventType: "TradeExecution",
      cdmJson: '{"event": "TradeExecution", "uti": "UTI-2026-US987654-001"}',
    };
    const cdmSpill = formatCdmSpill(cdmResp);
    expect(cdmSpill[1]).toEqual(["Execution ID", 987654]);
    expect(cdmSpill[2]).toEqual(["Unique Trade Identifier (UTI)", "UTI-2026-US987654-001"]);

    const attResp: AttestationResponse = {
      valid: true,
      attestationTimestampNanos: 1772841600000000000n,
      hardwareFingerprint: "PCR-SHA256-0x9F3E4A",
      statusMessage: "TPM 2.0 PCR Quote cryptographically verified",
    };
    const attSpill = formatAttestationSpill(attResp);
    expect(attSpill[0]).toEqual(["Status", "VALID"]);
    expect(attSpill[1]).toEqual(["Hardware Fingerprint", "PCR-SHA256-0x9F3E4A"]);

    const licResp: LicenseCapabilityResponse = {
      valid: true,
      subject: "institutional-tier-1",
      tier: "TIER_ENTERPRISE_GLOBAL",
      activeCapabilities: ["PRICING_ADVANCED", "ISDA_SIMM", "RAFT_CLUSTER"],
      expiryEpochSecs: 1893456000n,
    };
    const licSpill = formatLicenseCapabilitiesSpill(licResp);
    expect(licSpill[0]).toEqual(["Property", "Value"]);
    expect(licSpill[2]).toEqual(["License Subject", "institutional-tier-1"]);
    expect(licSpill[3]).toEqual(["License Tier", "TIER_ENTERPRISE_GLOBAL"]);
  });

  it("dispatches all capability RPCs through Connection", async () => {
    let sock: FakeSocket | null = null;
    const conn = new Connection({
      url: "ws://127.0.0.1:8081",
      factory: () => {
        sock = new FakeSocket();
        return sock;
      },
    });
    sock!.open();

    // 1. Calculate margin RPC
    const pMargin = conn.calculateMargin({ portfolio_id: "PORT-1" });
    const marginFrames = sock!.sentOfType("calculate_margin");
    expect(marginFrames.length).toBe(1);
    sock!.deliver({
      type: "calculate_margin_response",
      correlation_id: marginFrames[0]!.correlation_id,
      portfolio_id: "PORT-1",
      total_initial_margin: 250000,
    });
    const rMargin = await pMargin;
    expect(rMargin["total_initial_margin"]).toBe(250000);

    // 2. Pre-trade margin RPC
    const pPreTrade = conn.simulatePreTradeMargin({ portfolio_id: "PORT-1" });
    const preTradeFrames = sock!.sentOfType("simulate_pre_trade_margin");
    expect(preTradeFrames.length).toBe(1);
    sock!.deliver({
      type: "simulate_pre_trade_margin_response",
      correlation_id: preTradeFrames[0]!.correlation_id,
      outcome: "APPROVED",
      delta_margin: 50000,
    });
    const rPreTrade = await pPreTrade;
    expect(rPreTrade["outcome"]).toBe("APPROVED");

    // 3. Submit algo order RPC
    const pAlgo = conn.submitAlgoOrder({ symbol: "EURUSD", total_quantity: 1000000 });
    const algoFrames = sock!.sentOfType("submit_algo_order");
    expect(algoFrames.length).toBe(1);
    sock!.deliver({
      type: "submit_algo_order_response",
      correlation_id: algoFrames[0]!.correlation_id,
      parent_order_id: "ORDER-ALGO-1",
    });
    const rAlgo = await pAlgo;
    expect(rAlgo["parent_order_id"]).toBe("ORDER-ALGO-1");

    // 4. Cluster topology RPC
    const pCluster = conn.getClusterTopology();
    const clusterFrames = sock!.sentOfType("get_cluster_topology");
    expect(clusterFrames.length).toBe(1);
    sock!.deliver({
      type: "get_cluster_topology_response",
      correlation_id: clusterFrames[0]!.correlation_id,
      cluster_id: "cluster-1",
    });
    const rCluster = await pCluster;
    expect(rCluster["cluster_id"]).toBe("cluster-1");

    conn.close();
  });
});
