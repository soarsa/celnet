/**
 * GreeksStrip stories — Δ Γ ν Θ on the face, vanna/volga and the full 14-Greek
 * set on expand (GUI-DESIGN §3.5). Tabular, sign-aware, glyph-labelled. The strip
 * relabels the two rate-rho Greeks per asset class (FX/metal two-rate pair;
 * equity rate+dividend; commodity rate+net-carry; crypto rate+funding) — stories
 * show each class so the relabelling is reviewable. Token colors only — no raw hex.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { GreeksStrip } from "./GreeksStrip";
import type { Greeks } from "../data/contract";
import type { AssetClass } from "../products/types";

/** Synthetic Greeks representative of an ATM EUR/USD vanilla option. */
const FX_GREEKS: Greeks = {
  price: 0.00842,
  deltaSpot: 0.4921,
  deltaForward: 0.5003,
  gamma: 0.000312,
  vega: 0.00421,
  theta: -0.000185,
  rhoDom: 0.00218,
  rhoFor: -0.00201,
  vanna: 0.000042,
  volga: 0.000018,
  charm: -0.0000124,
  speed: 0.0000031,
  zomma: 0.0000087,
  color: 0.0000002,
};

/** Synthetic Greeks for an ATM equity vanilla option (SPX). */
const EQUITY_GREEKS: Greeks = {
  price: 12.45,
  deltaSpot: 0.5021,
  deltaForward: 0.5140,
  gamma: 0.000892,
  vega: 0.1842,
  theta: -0.00924,
  rhoDom: 0.08421,
  rhoFor: 0.06214,   // dividend-yield rho for equities
  vanna: 0.00092,
  volga: 0.00041,
  charm: -0.000384,
  speed: 0.0000841,
  zomma: 0.000214,
  color: 0.0000048,
};

/** Synthetic Greeks for an ATM commodity vanilla option (WTI). */
const COMMODITY_GREEKS: Greeks = {
  price: 3.82,
  deltaSpot: 0.4870,
  deltaForward: 0.5010,
  gamma: 0.000221,
  vega: 0.05814,
  theta: -0.003142,
  rhoDom: 0.02841,
  rhoFor: 0.01924,   // net-carry rho for commodities
  vanna: 0.000321,
  volga: 0.000142,
  charm: -0.0001521,
  speed: 0.0000312,
  zomma: 0.0000892,
  color: 0.0000018,
};

/** Synthetic Greeks for an ATM crypto vanilla option (BTC). */
const CRYPTO_GREEKS: Greeks = {
  price: 2841.0,
  deltaSpot: 0.5124,
  deltaForward: 0.5218,
  gamma: 0.0000021,
  vega: 84.21,
  theta: -3.841,
  rhoDom: 12.41,
  rhoFor: 8.214,   // funding rho for crypto
  vanna: 0.00841,
  volga: 0.00421,
  charm: -0.0841,
  speed: 0.0000182,
  zomma: 0.0000421,
  color: 0.0000009,
};

const ASSET_CLASSES: AssetClass[] = ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"];

const meta = {
  title: "Components/GreeksStrip",
  component: GreeksStrip,
  tags: ["autodocs"],
  argTypes: {
    assetClass: {
      control: "inline-radio",
      options: ASSET_CLASSES,
      description:
        "Relabels the two rate-rho Greeks to the real carry identity of the asset class. All 14 Greeks are always shown — only the label changes.",
    },
  },
  args: {
    greeks: FX_GREEKS,
    assetClass: "FX",
  },
} satisfies Meta<typeof GreeksStrip>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Default — FX class, collapsed (Δ Γ ν Θ visible). Click "full Greeks" to expand. */
export const Default: Story = {};

/** FX / Metal — domestic rate ρd and foreign rate ρf. */
export const FxAssetClass: Story = {
  args: { greeks: FX_GREEKS, assetClass: "FX" },
};

/** Equity — ρ (rate) and ρq (dividend yield). */
export const EquityAssetClass: Story = {
  args: { greeks: EQUITY_GREEKS, assetClass: "EQUITY" },
};

/** Commodity — ρ (rate) and ρc (net carry). */
export const CommodityAssetClass: Story = {
  args: { greeks: COMMODITY_GREEKS, assetClass: "COMMODITY" },
};

/** Crypto — ρ (rate) and ρƒ (funding). */
export const CryptoAssetClass: Story = {
  args: { greeks: CRYPTO_GREEKS, assetClass: "CRYPTO" },
};

/**
 * All asset classes side-by-side — confirms the ρ relabelling is distinctive
 * and readable across the full class matrix.
 */
export const AllAssetClasses: Story = {
  render: () => {
    const fixtures: { cls: AssetClass; greeks: Greeks }[] = [
      { cls: "FX", greeks: FX_GREEKS },
      { cls: "EQUITY", greeks: EQUITY_GREEKS },
      { cls: "COMMODITY", greeks: COMMODITY_GREEKS },
      { cls: "CRYPTO", greeks: CRYPTO_GREEKS },
    ];
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-6)",
          padding: "var(--space-5)",
          background: "var(--bg-base)",
          borderRadius: "var(--r-md)",
        }}
      >
        {fixtures.map(({ cls, greeks }) => (
          <div key={cls}>
            <div
              style={{
                fontSize: "var(--type-caption)",
                color: "var(--text-tertiary)",
                textTransform: "uppercase",
                letterSpacing: "0.06em",
                marginBottom: "var(--space-2)",
              }}
            >
              {cls}
            </div>
            <GreeksStrip greeks={greeks} assetClass={cls} />
          </div>
        ))}
      </div>
    );
  },
};
