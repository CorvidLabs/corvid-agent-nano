/**
 * Plugin Bridge — TypeScript client for the Rust plugin host sidecar.
 *
 * Communicates over Unix domain socket using newline-delimited JSON-RPC.
 * Auto-registers plugin tools into the agent tool registry on connect.
 * Reconnects with exponential backoff if the socket drops.
 * Handles push notifications for hot-reload of individual plugin tools.
 */

import { connect, type Socket } from "node:net";
import { encode } from "@msgpack/msgpack";

// ── Public types (camelCase, TypeScript conventions) ───────────────────

export interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];
  trustTier: "trusted" | "verified" | "untrusted";
  tools: ToolInfo[];
}

export interface ToolInfo {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

export interface HealthStatus {
  connected: boolean;
  plugins: Record<string, "active" | "draining" | "unloaded">;
  uptimeMs: number;
}

export interface PluginEvent {
  type: string;
  pluginId?: string;
  payload: unknown;
}

// ── Wire types (snake_case, matching Rust serialization) ───────────────

interface WirePluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];
  trust_tier: "trusted" | "verified" | "untrusted";
  tools: WireToolInfo[];
}

interface WireToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

// ── JSON-RPC types ─────────────────────────────────────────────────────

interface JsonRpcRequest {
  method: string;
  params: unknown;
  id: number;
}

interface JsonRpcResponse {
  result?: unknown;
  error?: string;
  id: number | null;
}

interface JsonRpcNotification {
  event: string;
  pluginId?: string;
  trust_tier?: string;
  tools?: WireToolInfo[];
}

type ToolRegistry = {
  register(entry: {
    name: string;
    description: string;
    inputSchema: Record<string, unknown>;
    execute: (input: unknown) => Promise<string>;
  }): void;
  unregister(name: string): void;
};

// ── Timeouts per trust tier ────────────────────────────────────────────

const INVOKE_TIMEOUT: Record<string, number> = {
  trusted: 30_000,
  verified: 5_000,
  untrusted: 1_000,
};

// ── Security constants ─────────────────────────────────────────────────

/** Plugin IDs must match the host's manifest ID regex. */
const PLUGIN_ID_RE = /^[a-z][a-z0-9-]{0,49}$/;

/** Tool names: lowercase letters, digits, hyphens, underscores. */
const TOOL_NAME_RE = /^[a-z][a-z0-9_-]{0,63}$/;

// ── Bridge ─────────────────────────────────────────────────────────────

export class PluginBridge {
  private socket: Socket | null = null;
  private socketPath = "";
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  private buffer = "";
  private reconnectMs = 500;
  private reconnectMax: number;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private closed = false;
  private toolRegistry: ToolRegistry | null = null;
  private registeredTools = new Set<string>();
  private pluginToolsByPlugin = new Map<string, Set<string>>();
  private pluginTiers = new Map<string, string>();

  constructor(opts?: { reconnectMax?: number; toolRegistry?: ToolRegistry }) {
    this.reconnectMax = opts?.reconnectMax ?? 30_000;
    this.toolRegistry = opts?.toolRegistry ?? null;
  }

  /** Connect to the plugin host Unix socket. */
  async connect(socketPath: string): Promise<void> {
    this.socketPath = socketPath;
    this.closed = false;
    return this.doConnect();
  }

  /** Gracefully close the socket connection. */
  async disconnect(): Promise<void> {
    this.closed = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.unregisterAllTools();
    if (this.socket) {
      this.socket.destroy();
      this.socket = null;
    }
    // Reject all pending requests
    for (const [, { reject }] of this.pending) {
      reject(new Error("bridge disconnected"));
    }
    this.pending.clear();
  }

  /** List all loaded plugin manifests. */
  async listManifests(): Promise<PluginManifest[]> {
    const resp = await this.rpc("plugin.list", {});
    const data = resp as { plugins?: WirePluginManifest[] };
    return (data.plugins ?? []).map(mapManifest);
  }

  /** List tools (all or filtered by plugin). */
  async listTools(pluginId?: string): Promise<ToolInfo[]> {
    const params = pluginId ? { id: pluginId } : {};
    const resp = await this.rpc("plugin.tools", params);
    const data = resp as { tools?: Array<{ plugin_id: string; tool: WireToolInfo }> };
    return (data.tools ?? []).map((t) => mapTool(t.tool));
  }

  /** Invoke a plugin tool. Uses MessagePack for the payload. */
  async invoke(pluginId: string, tool: string, input: unknown): Promise<string> {
    if (!PLUGIN_ID_RE.test(pluginId)) {
      throw new Error(`invalid plugin ID: ${pluginId}`);
    }
    if (!TOOL_NAME_RE.test(tool)) {
      throw new Error(`invalid tool name: ${tool}`);
    }

    const tier = this.pluginTiers.get(pluginId) ?? "untrusted";
    const timeout = INVOKE_TIMEOUT[tier] ?? INVOKE_TIMEOUT.untrusted;

    const payload = encode({ pluginId, tool, input });
    const resp = await this.rpcWithTimeout(
      "plugin.invoke",
      { pluginId, tool, input: Buffer.from(payload).toString("base64") },
      timeout,
    );

    const data = resp as { result?: string; error?: unknown; unavailable?: boolean };
    if (data.unavailable) {
      throw Object.assign(new Error(`plugin ${pluginId} is draining`), { status: 503, retryable: true });
    }
    if (data.error != null) {
      const msg = typeof data.error === "string" ? data.error : `plugin error (${pluginId}:${tool})`;
      throw new Error(msg);
    }
    return data.result ?? "";
  }

  /** Forward an event to subscribing plugins. */
  async dispatchEvent(event: PluginEvent): Promise<void> {
    await this.rpc("plugin.dispatch", event);
  }

  /** Check plugin host health. */
  async healthCheck(): Promise<HealthStatus> {
    try {
      const resp = await this.rpc("health.check", {});
      const data = resp as { plugins: Record<string, string>; uptime_ms: number };
      return {
        connected: true,
        plugins: data.plugins as HealthStatus["plugins"],
        uptimeMs: data.uptime_ms,
      };
    } catch {
      return { connected: false, plugins: {}, uptimeMs: 0 };
    }
  }

  /** Whether the bridge is currently connected. */
  get connected(): boolean {
    return this.socket !== null && !this.socket.destroyed;
  }

  // ── Auto-registration ──────────────────────────────────────────────

  /** Refresh tool registry from host — called on connect and can be called manually. */
  async refreshTools(): Promise<void> {
    if (!this.toolRegistry) return;

    // Unregister all stale tools
    this.unregisterAllTools();

    // Fetch manifests to get tier info
    const manifests = await this.listManifests();
    for (const m of manifests) {
      this.pluginTiers.set(m.id, m.trustTier);
    }

    // Fetch all tools and register them
    const resp = await this.rpc("plugin.tools", {});
    const data = resp as { tools?: Array<{ plugin_id: string; tool: WireToolInfo }> };
    for (const entry of data.tools ?? []) {
      this.registerPluginTool(entry.plugin_id, entry.tool);
    }
  }

  /** Refresh tools for a single plugin (hot-reload). Only touches that plugin's tools. */
  private refreshPluginTools(pluginId: string, rawTools: WireToolInfo[]): void {
    if (!this.toolRegistry) return;

    // Unregister old tools for this plugin only
    const oldTools = this.pluginToolsByPlugin.get(pluginId) ?? new Set();
    for (const toolName of oldTools) {
      this.toolRegistry.unregister(toolName);
      this.registeredTools.delete(toolName);
    }
    this.pluginToolsByPlugin.delete(pluginId);

    // Register new tools
    for (const tool of rawTools) {
      this.registerPluginTool(pluginId, tool);
    }
  }

  private registerPluginTool(pluginId: string, rawTool: WireToolInfo): void {
    if (!this.toolRegistry) return;
    const toolName = `plugin:${pluginId}:${rawTool.name}`;
    this.toolRegistry.register({
      name: toolName,
      description: rawTool.description,
      inputSchema: rawTool.input_schema,
      execute: (input) => this.invoke(pluginId, rawTool.name, input),
    });
    this.registeredTools.add(toolName);
    const pluginTools = this.pluginToolsByPlugin.get(pluginId) ?? new Set();
    pluginTools.add(toolName);
    this.pluginToolsByPlugin.set(pluginId, pluginTools);
  }

  private unregisterAllTools(): void {
    if (!this.toolRegistry) return;
    for (const name of this.registeredTools) {
      this.toolRegistry.unregister(name);
    }
    this.registeredTools.clear();
    this.pluginToolsByPlugin.clear();
  }

  // ── JSON-RPC transport ─────────────────────────────────────────────

  private rpc(method: string, params: unknown): Promise<unknown> {
    return this.rpcWithTimeout(method, params, 10_000);
  }

  private rpcWithTimeout(method: string, params: unknown, timeoutMs: number): Promise<unknown> {
    return new Promise((resolve, reject) => {
      if (!this.socket || this.socket.destroyed) {
        reject(new Error("not connected to plugin host"));
        return;
      }

      const id = this.nextId++;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`RPC timeout: ${method} (${timeoutMs}ms)`));
      }, timeoutMs);

      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });

      const req: JsonRpcRequest = { method, params, id };
      this.socket.write(JSON.stringify(req) + "\n");
    });
  }

  // Maximum receive buffer size (1 MiB). Protects against a malicious or
  // buggy plugin host that sends a huge message without a newline delimiter.
  private static readonly MAX_BUFFER_BYTES = 1_048_576;

  private handleData(chunk: string): void {
    this.buffer += chunk;

    if (this.buffer.length > PluginBridge.MAX_BUFFER_BYTES) {
      console.error("[plugin-bridge] receive buffer exceeded limit — dropping connection");
      this.socket?.destroy();
      return;
    }

    let newlineIdx: number;
    while ((newlineIdx = this.buffer.indexOf("\n")) !== -1) {
      const line = this.buffer.slice(0, newlineIdx);
      this.buffer = this.buffer.slice(newlineIdx + 1);

      if (!line.trim()) continue;

      try {
        const msg = JSON.parse(line) as Record<string, unknown>;

        // Server-pushed notification: has "event" field, no numeric id
        if (typeof msg.event === "string") {
          this.handleNotification(msg as unknown as JsonRpcNotification);
          continue;
        }

        // RPC response
        const resp = msg as unknown as JsonRpcResponse;
        if (resp.id != null && this.pending.has(resp.id)) {
          const { resolve, reject } = this.pending.get(resp.id)!;
          this.pending.delete(resp.id);
          if (resp.error) {
            reject(new Error(resp.error));
          } else {
            resolve(resp.result);
          }
        }
      } catch {
        // Ignore malformed lines
      }
    }
  }

  private handleNotification(notification: JsonRpcNotification): void {
    if (notification.event === "plugin.tools_registered") {
      const { pluginId, tools } = notification;
      if (!pluginId || !tools) return;

      // Update tier from notification if provided
      if (notification.trust_tier) {
        this.pluginTiers.set(pluginId, notification.trust_tier);
      }

      this.refreshPluginTools(pluginId, tools);
    }
  }

  // ── Connection management ──────────────────────────────────────────

  private doConnect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const socket = connect({ path: this.socketPath });

      socket.on("connect", () => {
        this.socket = socket;
        this.buffer = "";
        this.reconnectMs = 500;

        socket.setEncoding("utf-8");
        socket.on("data", (data) => this.handleData(data as string));

        // Auto-register tools on connect — suppress warning if bridge closed before completing
        this.refreshTools().catch((err) => {
          if (!this.closed) {
            console.warn("[plugin-bridge] tool refresh failed:", err.message);
          }
        });

        resolve();
      });

      socket.on("error", (err) => {
        if (!this.socket) {
          // Initial connection failed
          console.warn(`[plugin-bridge] connection refused: ${this.socketPath}`);
          reject(err);
          this.scheduleReconnect();
          return;
        }
      });

      socket.on("close", () => {
        this.socket = null;
        // Reject pending requests
        for (const [, { reject: rej }] of this.pending) {
          rej(new Error("socket closed"));
        }
        this.pending.clear();
        this.unregisterAllTools();

        if (!this.closed) {
          this.scheduleReconnect();
        }
      });
    });
  }

  private scheduleReconnect(): void {
    if (this.closed || this.reconnectTimer) return;

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.doConnect().catch(() => {
        // Exponential backoff
        this.reconnectMs = Math.min(this.reconnectMs * 2, this.reconnectMax);
      });
    }, this.reconnectMs);
  }
}

// ── Wire → public type mapping ─────────────────────────────────────────

function mapManifest(wire: WirePluginManifest): PluginManifest {
  return {
    id: wire.id,
    version: wire.version,
    author: wire.author,
    description: wire.description,
    capabilities: wire.capabilities,
    trustTier: wire.trust_tier,
    tools: wire.tools.map(mapTool),
  };
}

function mapTool(wire: WireToolInfo): ToolInfo {
  return {
    name: wire.name,
    description: wire.description,
    inputSchema: wire.input_schema,
  };
}
