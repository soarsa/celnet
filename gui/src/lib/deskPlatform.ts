/**
 * CelNet Desk Architecture & Platform Bridge
 *
 * Implements the Desk Platform Architecture contract connecting CelNet's
 * React 19 GUI tier to DeskModal's Rust+Tauri desktop agent and FDC3 2.2 broker.
 *
 * Conforms to:
 * - DeskModal SDK Boundary Doctrine (core.md §5, sdks-overview.md)
 * - FDC3 2.2 Financial Desktop Connectivity Specification
 * - Zero quantitative duplication doctrine (Rust engines compute, TS renders)
 *
 * Provides:
 * - Runtime detection of DeskModal Desktop Agent (`isDeskModal`)
 * - Typed access to DeskModal host APIs (`getDeskModal`)
 * - Native window management and title sync (no custom chrome)
 * - Cross-app reactive state synchronization via SQLite WAL shared storage
 * - Zero-copy streaming market data and valuation broadcasts
 * - DeskModal status bar item registration and notification dispatch
 */

export interface DeskStatusBarItem {
  id: string;
  label: string;
  status: "success" | "warning" | "error" | "neutral";
  tooltip?: string;
}

export interface DeskWorkspaceTemplate {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
}

export interface DeskModalHostAPI {
  version: string;
  streamingBroadcast(channel: unknown, context: unknown): void;
  auth?: {
    getToken(): Promise<string | null>;
    getIdentity(): Promise<{ sub: string; email?: string; name?: string } | null>;
  };
  telemetry?: {
    emit(tag: string, payload: unknown): Promise<void>;
  };
  sharedStorage: {
    get<T>(key: string): Promise<T | null>;
    set<T>(key: string, value: T): Promise<void>;
    delete(key: string): Promise<void>;
    keys(): Promise<string[]>;
  };
  statusBar: {
    setItems(items: DeskStatusBarItem[]): void;
    clearItems(): void;
  };
  windowMode: "standalone" | "pane" | "dock";
  workspace: {
    save(name: string): Promise<void>;
    load(name: string): Promise<void>;
    getTemplates(): Promise<DeskWorkspaceTemplate[]>;
  };
  notification: {
    send(
      title: string,
      body: string,
      options?: {
        priority?: "critical" | "normal" | "low";
        sourceApp?: string;
        category?: string;
        indicatorColor?: string;
        duration?: number;
        actions?: Array<{ label: string; actionType: string; payload?: string }>;
      }
    ): Promise<string>;
    dismiss(id: string): Promise<void>;
  };
  shortcuts: {
    register(appId: string, shortcut: string, action: string, scope?: "AppFocused" | "Global"): Promise<void>;
    unregister(appId: string, shortcut: string): Promise<void>;
    list(): Promise<Array<{ shortcut: string; action: string; appId: string; scope: string }>>;
  };
  toolbar?: {
    registerWidget(
      appId: string,
      widgetId: string,
      config: {
        type: "Dropdown" | "Toggle" | "Label" | "Button";
        position: "Left" | "Center" | "Right";
        label?: string;
        value?: string;
        enabled?: boolean;
      }
    ): Promise<void>;
    updateWidget(
      appId: string,
      widgetId: string,
      update: Partial<{ label: string; value: string; enabled: boolean }>
    ): Promise<void>;
  };
  appStorage?: {
    get(appId: string, key: string): Promise<string | null>;
    set(appId: string, key: string, value: string): Promise<void>;
    delete(appId: string, key: string): Promise<void>;
    list(appId: string): Promise<string[]>;
  };
  audit?: {
    record(
      eventType: string,
      appId: string,
      details: string,
      severity?: "info" | "warning" | "error"
    ): Promise<string>;
  };
  a11y?: {
    announce(message: string, priority?: "polite" | "assertive"): Promise<void>;
    getMotionPrefs(): Promise<{ reducedMotion: boolean }>;
    getScale(): Promise<{ systemScale: number; remBasePx: number }>;
  };
  dlp?: {
    classify(context: unknown): Promise<"Public" | "Internal" | "Confidential" | "Restricted">;
    setScreenShareProtection(windowId: string, enabled: boolean): Promise<void>;
  };
  windowGroup?: {
    create(windowIds: string[]): Promise<string>;
    add(groupId: string, windowId: string): Promise<void>;
    remove(windowId: string): Promise<void>;
    move(windowId: string, x: number, y: number, w: number, h: number): Promise<void>;
  };
}

/** Check if currently running inside DeskModal desktop agent. */
export function isDeskModal(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as {
    deskmodal?: { version?: string; auth?: unknown; shell?: unknown; telemetry?: unknown };
    fdc3?: unknown;
    __TAURI__?: unknown;
    __DESKMODAL_PRELOAD_VERSION__?: unknown;
  };
  return (
    (typeof w.deskmodal !== "undefined" &&
      (typeof w.deskmodal.version === "string" ||
        typeof w.deskmodal.auth !== "undefined" ||
        typeof w.deskmodal.shell !== "undefined")) ||
    typeof w.__DESKMODAL_PRELOAD_VERSION__ !== "undefined" ||
    (typeof w.fdc3 !== "undefined" &&
      typeof window.location !== "undefined" &&
      window.location.protocol === "deskmodal-plugin:") ||
    (typeof window.location !== "undefined" &&
      window.location.protocol === "deskmodal-plugin:")
  );
}

/**
 * Detect if running as a dedicated micro-app (Tile or Tearout window in DeskModal,
 * or with ?mode=app / ?appMode).
 */
export function isAppMode(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as {
    __CELNET_APP_MODE__?: boolean;
    __TAURI__?: unknown;
    __DESKMODAL_PRELOAD_VERSION__?: unknown;
    deskmodal?: { windowMode?: string };
  };
  if (w.__CELNET_APP_MODE__ === true) return true;

  try {
    const params = new URLSearchParams(window.location.search);
    if (params.get("mode") === "app" || params.has("appMode")) return true;
    if (params.get("mode") === "shell") return false;

    // Check pathname: if launched directly from a desk sub-folder (/pricing, /markets, etc.)
    const path = window.location.pathname.toLowerCase();
    if (
      path.includes("/pricing") ||
      path.includes("/markets") ||
      path.includes("/rfq") ||
      path.includes("/distribution") ||
      path.includes("/risk") ||
      path.includes("/blotter") ||
      path.includes("/analytics") ||
      path.includes("/policy")
    ) {
      return true;
    }

    // Check DeskModal container protocol or iframe embedding
    if (window.location.protocol === "deskmodal-plugin:") return true;
    if (window.parent && window.parent !== window) return true;
    if (w.deskmodal?.windowMode === "pane" || w.deskmodal?.windowMode === "dock") return true;
  } catch {}

  return false;
}

/**
 * Get the isolated Instance ID for this window (e.g. from ?instanceId=... or window name).
 */
export function getInstanceId(): string {
  if (typeof window === "undefined") return "main";
  try {
    const p = new URLSearchParams(window.location.search);
    const id = p.get("instanceId") || p.get("inst");
    if (id) return id;
    if (window.name && window.name.startsWith("celnet-")) return window.name;
  } catch {}
  return "default";
}

/** Get the typed DeskModal Host API instance, or null if outside DeskModal. */
export function getDeskModal(): DeskModalHostAPI | null {
  if (!isDeskModal()) return null;
  return (window as unknown as { deskmodal: DeskModalHostAPI }).deskmodal;
}

/** Retrieve active authentication token from host container or deployment environment. */
export async function getDeskModalAuthToken(): Promise<string | null> {
  if (typeof window === "undefined") return null;
  const w = window as unknown as {
    __DESKMODAL_AUTH_TOKEN__?: string;
    deskmodal?: {
      auth?: {
        getToken?: () => Promise<string | null>;
      };
    };
  };

  // 1. Direct host injection via window global
  if (w.__DESKMODAL_AUTH_TOKEN__) {
    return w.__DESKMODAL_AUTH_TOKEN__;
  }

  // 2. URL search parameters (Enterprise Gateway / OAuth2 redirect / Container injection)
  try {
    const urlParams = new URLSearchParams(window.location.search);
    const paramToken = urlParams.get("token") || urlParams.get("jwt") || urlParams.get("access_token");
    if (paramToken && paramToken.length > 20) {
      return paramToken;
    }
  } catch {}

  // 3. DeskModal typed host API
  if (w.deskmodal?.auth?.getToken) {
    try {
      const t = await w.deskmodal.auth.getToken();
      if (t) return t;
    } catch {}
  }

  // 4. DACP / postMessage bridge when running inside a DeskModal tearout iframe
  if (typeof window.parent !== "undefined" && window.parent !== window) {
    try {
      const dacpToken = await new Promise<string | null>((resolve) => {
        const reqId = "auth-token-" + Math.random().toString(36).slice(2, 9);
        const timer = setTimeout(() => {
          window.removeEventListener("message", onMsg);
          resolve(null);
        }, 600);

        function onMsg(ev: MessageEvent) {
          const data = ev.data as {
            type?: string;
            payload?: { result?: { access_token?: string }; error?: string };
            meta?: { requestUuid?: string };
          };
          if (data && data.type === "authGetToken" && data.meta?.requestUuid === reqId) {
            clearTimeout(timer);
            window.removeEventListener("message", onMsg);
            if (data.payload?.result?.access_token) {
              resolve(data.payload.result.access_token);
            } else {
              resolve(null);
            }
          }
        }

        window.addEventListener("message", onMsg);
        window.parent.postMessage(
          {
            type: "authGetToken",
            payload: { appId: "celnet.studio" },
            meta: { requestUuid: reqId },
          },
          "*",
        );
      });

      if (dacpToken) return dacpToken;
    } catch {}
  }

  // 5. Fallback: resolve from plugin asset storage
  const candidatePaths = [
    "./assets/jwt.json",
    "../assets/jwt.json",
    "/celnet.studio/app/assets/jwt.json",
  ];
  for (const path of candidatePaths) {
    try {
      const res = await fetch(path);
      if (res.ok) {
        const raw = await res.json();
        if (typeof raw === "string" && raw.length > 20) {
          return raw;
        }
      }
    } catch {}
  }

  return null;
}

/** Retrieve active user identity from DeskModal host agent. */
export async function getDeskModalAuthIdentity(): Promise<{ sub: string; email?: string; name?: string } | null> {
  if (typeof window === "undefined") return null;
  const w = window as unknown as {
    __DESKMODAL_AUTH_USER__?: { sub: string; email?: string; name?: string };
    deskmodal?: {
      auth?: {
        getIdentity?: () => Promise<{ sub: string; email?: string; name?: string } | null>;
      };
    };
  };

  if (w.__DESKMODAL_AUTH_USER__) {
    return w.__DESKMODAL_AUTH_USER__;
  }

  if (w.deskmodal?.auth?.getIdentity) {
    try {
      const id = await w.deskmodal.auth.getIdentity();
      if (id) return id;
    } catch {}
  }

  // Fallback: resolve from plugin asset storage
  const candidatePaths = [
    "./assets/user.json",
    "../assets/user.json",
    "/celnet.studio/app/assets/user.json",
  ];
  for (const path of candidatePaths) {
    try {
      const res = await fetch(path);
      if (res.ok) {
        const raw = (await res.json()) as { id?: string; sub?: string; email?: string; name?: string };
        if (raw) {
          const idObj: { sub: string; email?: string; name?: string } = {
            sub: raw.sub || raw.id || "unknown",
          };
          if (typeof raw.email === "string") idObj.email = raw.email;
          if (typeof raw.name === "string") idObj.name = raw.name;
          return idObj;
        }
      }
    } catch {}
  }

  return null;
}

/** Emit a consolidated structured telemetry / audit record to DeskModal host. */
export function emitDeskTelemetry(tag: string, payload: Record<string, unknown>): void {
  if (typeof window === "undefined") return;
  const w = window as unknown as {
    deskmodal?: {
      telemetry?: {
        emit?: (tag: string, payload: unknown) => Promise<void>;
      };
    };
  };
  if (w.deskmodal?.telemetry?.emit) {
    w.deskmodal.telemetry.emit(tag, payload).catch(() => {});
  }
}

/** Set the OS-native window title, coordinating with DeskModal / Tauri. */
export function setDeskWindowTitle(title: string): void {
  if (typeof document !== "undefined") {
    document.title = title;
  }
  if (typeof window !== "undefined") {
    const w = window as unknown as {
      __TAURI__?: { window?: { getCurrentWindow?: () => { setTitle?: (t: string) => Promise<void> } } };
    };
    if (w.__TAURI__?.window?.getCurrentWindow?.()?.setTitle) {
      w.__TAURI__.window.getCurrentWindow()!.setTitle!(title).catch(() => {});
    }
  }
}

/** Publish a shared state key across all DeskModal windows with local fallback. */
export async function setSharedState<T>(key: string, value: T): Promise<void> {
  const desk = getDeskModal();
  if (desk?.sharedStorage?.set) {
    try {
      await desk.sharedStorage.set(key, value);
      return;
    } catch (err) {
      console.warn("[DeskPlatform] sharedStorage.set failed, using localStorage:", err);
    }
  }
  if (typeof localStorage !== "undefined") {
    try {
      localStorage.setItem(`celnet_shared_${key}`, JSON.stringify(value));
    } catch {}
  }
}

/** Retrieve a shared state key across all DeskModal windows with local fallback. */
export async function getSharedState<T>(key: string, defaultValue: T): Promise<T> {
  const desk = getDeskModal();
  if (desk?.sharedStorage?.get) {
    try {
      const val = await desk.sharedStorage.get<T>(key);
      if (val !== null && val !== undefined) return val;
    } catch (err) {
      console.warn("[DeskPlatform] sharedStorage.get failed, using localStorage:", err);
    }
  }
  if (typeof localStorage !== "undefined") {
    try {
      const raw = localStorage.getItem(`celnet_shared_${key}`);
      if (raw !== null) return JSON.parse(raw) as T;
    } catch {}
  }
  return defaultValue;
}

/** Send an institutional notification through DeskModal notification center or browser Notification. */
export async function notifyTrader(
  title: string,
  body: string,
  priority: "critical" | "normal" | "low" = "normal"
): Promise<void> {
  const desk = getDeskModal();
  if (desk?.notification?.send) {
    try {
      await desk.notification.send(title, body, {
        priority,
        sourceApp: "celnet.studio",
        category: "Trading",
      });
      return;
    } catch {}
  }
  if (typeof window !== "undefined" && "Notification" in window && Notification.permission === "granted") {
    new Notification(title, { body });
  }
}
