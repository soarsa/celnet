/**
 * The per-connection FIX client-config generator: the artifact an operator hands
 * a counterparty so they can build a price-taking client against an acceptor.
 *
 * The load-bearing correctness property is the CompID swap — a client's
 * SenderCompID is the venue's TargetCompID and vice-versa (the acceptor refuses a
 * peer that does not mirror its CompIDs). Also covers the bind-address parse
 * (including the wildcard-host → loopback fallback) and the dictionary reference.
 */

import { describe, expect, it } from "vitest";

import type { FixConnection } from "../src/data/contract";
import {
  buildFixClientConfig,
  fixClientConfigFilename,
  FIX_DICTIONARY_FILENAME,
} from "../src/lib/fixClientConfig";

function conn(overrides: Partial<FixConnection> = {}): FixConnection {
  return {
    id: "demo-options",
    name: "Demo bank — Options",
    kind: "OPTIONS",
    bindAddr: "127.0.0.1:9099",
    senderCompId: "CELNET",
    targetCompId: "CELNET-CPTY",
    enabled: true,
    running: true,
    boundAddr: "127.0.0.1:9099",
    desk: "g10",
    ...overrides,
  };
}

describe("buildFixClientConfig", () => {
  it("swaps the CompIDs (client = mirror of the venue)", () => {
    const cfg = buildFixClientConfig(conn());
    // The venue's TargetCompID becomes the client's SenderCompID, and vice-versa.
    expect(cfg).toContain("SenderCompID=CELNET-CPTY");
    expect(cfg).toContain("TargetCompID=CELNET");
  });

  it("parses host and port from the bind address", () => {
    const cfg = buildFixClientConfig(conn({ bindAddr: "10.0.0.5:9300" }));
    expect(cfg).toContain("SocketConnectHost=10.0.0.5");
    expect(cfg).toContain("SocketConnectPort=9300");
  });

  it("falls back to loopback for a wildcard bind host, with a note", () => {
    const cfg = buildFixClientConfig(conn({ bindAddr: "0.0.0.0:9400" }));
    expect(cfg).toContain("SocketConnectHost=127.0.0.1");
    expect(cfg).toContain("SocketConnectPort=9400");
    expect(cfg).toContain("wildcard");
  });

  it("is FIX.4.4 and references the data dictionary", () => {
    const cfg = buildFixClientConfig(conn());
    expect(cfg).toContain("BeginString=FIX.4.4");
    expect(cfg).toContain(`DataDictionary=${FIX_DICTIONARY_FILENAME}`);
    expect(cfg).toContain("ConnectionType=initiator");
  });

  it("derives a filesystem-safe filename from the connection id", () => {
    expect(fixClientConfigFilename(conn({ id: "Bank A/Opt!" }))).toBe("celnet-fix-Bank-A-Opt.cfg");
    expect(fixClientConfigFilename(conn({ id: "demo-options" }))).toBe("celnet-fix-demo-options.cfg");
  });
});
