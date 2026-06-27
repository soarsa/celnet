/**
 * Storybook 8 — React + Vite builder, wired to the GUI's own vite.config.ts so
 * stories compile under the exact same plugin/resolve/`define` pipeline as the
 * app (no parallel build config to drift). ADDITIVE: this stands the component
 * gallery up alongside the existing app/test/e2e tooling without touching it.
 *
 * Aurora design system is layered in via .storybook/preview.ts, which imports
 * src/design/global.css (→ tokens.css) so every story renders against the real
 * token cascade.
 */

import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  // Co-located *.stories.tsx next to the components they document.
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  addons: ["@storybook/addon-essentials"],
  framework: {
    name: "@storybook/react-vite",
    options: {},
  },
  core: {
    disableTelemetry: true,
  },
  // Reuse the app's vite.config.ts (react plugin, es2022 target, build-stamp
  // `define`s) rather than maintaining a second config. The builder merges this
  // over Storybook's defaults.
  viteFinal: async (viteConfig) => {
    const { mergeConfig } = await import("vite");
    const { default: appConfig } = await import("../vite.config.ts");
    const resolved =
      typeof appConfig === "function"
        ? appConfig({ command: "build", mode: "production" })
        : appConfig;
    return mergeConfig(viteConfig, {
      plugins: resolved.plugins,
      define: resolved.define,
      resolve: resolved.resolve,
    });
  },
  typescript: {
    // react-docgen drives the auto-generated Controls/props table.
    reactDocgen: "react-docgen-typescript",
  },
};

export default config;
