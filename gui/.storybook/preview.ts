/**
 * Storybook preview — renders every story against the real Aurora cascade.
 *
 * The app drives theming through data attributes on <html> (see
 * src/design/appearance.ts: data-appearance = dark|light, data-contrast =
 * normal|high). We mirror that exactly here: a decorator stamps the chosen
 * attributes onto document.documentElement before each story, and a global
 * toolbar lets you flip appearance/contrast — so a story proves the component
 * in both poles (the design system treats light and dark as equals).
 */

import type { Decorator, Preview } from "@storybook/react";

// Pulls in src/design/global.css → tokens.css: the same stylesheet main.tsx
// loads, so stories share one source of truth for color/type/spacing.
import "../src/design/global.css";

const withAurora: Decorator = (Story, context) => {
  const root = document.documentElement;
  const appearance = (context.globals.appearance as string) ?? "dark";
  const contrast = (context.globals.contrast as string) ?? "normal";
  root.setAttribute("data-appearance", appearance);
  root.setAttribute("data-contrast", contrast === "high" ? "high" : "normal");
  return Story();
};

const preview: Preview = {
  decorators: [withAurora],
  globalTypes: {
    appearance: {
      description: "Aurora appearance",
      defaultValue: "dark",
      toolbar: {
        title: "Appearance",
        icon: "circlehollow",
        items: [
          { value: "dark", title: "Dark" },
          { value: "light", title: "Light" },
        ],
        dynamicTitle: true,
      },
    },
    contrast: {
      description: "Aurora contrast",
      defaultValue: "normal",
      toolbar: {
        title: "Contrast",
        icon: "contrast",
        items: [
          { value: "normal", title: "Normal" },
          { value: "high", title: "Increased" },
        ],
        dynamicTitle: true,
      },
    },
  },
  parameters: {
    layout: "centered",
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
  },
};

export default preview;
