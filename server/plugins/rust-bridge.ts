/**
 * Plugin Bridge — TypeScript client for the Rust plugin host sidecar.
 *
 * Communicates over Unix domain socket using:
 * - Control plane: JSON-RPC (newline-delimited)
 * - Data plane: MessagePack (base64-encoded in JSON-RPC params for tool invocation)
 *
 * Features:
 * - Auto-registers plugin tools into the agent tool registry on connect
 * - Enforces per-trust-tier timeouts on tool invocation (trusted: 30s, verified: 5s, untrusted: 1s)
 * - Reconnects with exponential backoff (500ms initial, configurable max ~30s) if the socket drops
 * - Handles socket lifecycle events (connect, close, error)
 *
 * When a PluginBridge is constructed with a toolRegistry option, tools are automatically
 * discovered and registered on connect() and when refreshTools() is called.
 */

import { connect, type Socket } from "node:net";
import { encode, decode } from "@msgpack/msgpack";

// ── Types ──────────────────────────────────────────────────────────────

export interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];
  trust_tier: "trusted" | "verified" | "untrusted";
  tools: ToolInfo[];
}

export interface ToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
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
  private pluginTiers = new Map<string, string>();

  /**
   * Initialize the plugin bridge.
   * @param opts.reconnectMax Maximum reconnect backoff in milliseconds (default: 30_000)
   * @param opts.toolRegistry Optional tool registry for auto-registration on connect/refresh
   */
  constructor(opts?: { reconnectMax?: number; toolRegistry?: ToolRegistry }) {
    this.reconnectMax = opts?.reconnectMax ?? 30_000;
    this.toolRegistry = opts?.toolRegistry ?? null;
  }

  /**
   * Connect to the plugin host Unix socket.
   * Initiates auto-registration via refreshTools() if a tool registry was provided.
   * @param socketPath Path to the Unix socket (e.g., ~/.corvid/plugins.sock)
   * @throws Error if connection is refused; schedules automatic reconnect with exponential backoff
   */
  async connect(socketPath: string): Promise<void> {
    this.socketPath = socketPath;
    this.closed = false;
    return this.doConnect();
  }

  /**
   * Gracefully close the socket connection.
   * Clears all timers, rejects pending requests, unregisters tools, and prevents reconnection.
   */
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

  /**
   * RPC `plugin.list` — List all loaded plugin manifests.
   * @returns Array of plugin manifests with metadata (id, version, author, description, capabilities, trust_tier, tools)
   */
  async listManifests(): Promise<PluginManifest[]> {
    const resp = await this.rpc("plugin.list", {});
    const data = resp as { plugins?: PluginManifest[] };
    return data.plugins ?? [];
  }

  /**
   * RPC `plugin.tools` — List all tools, optionally filtered by plugin.
   * @param pluginId Optional plugin ID to filter tools to that plugin only
   * @returns Array of tools with name, description, and JSON Schema v7 input_schema
   */
  async listTools(pluginId?: string): Promise<ToolInfo[]> {
    const params = pluginId ? { id: pluginId } : {};
    const resp = await this.rpc("plugin.tools", params);
    const data = resp as { tools?: Array<{ plugin_id: string; tool: ToolInfo }> };
    return (data.tools ?? []).map((t) => t.tool);
  }

  /**
   * RPC `plugin.invoke` — Invoke a plugin tool with MessagePack-encoded input.
   * Respects trust tier timeouts: trusted (30s), verified (5s), untrusted (1s).
   * @param pluginId Plugin ID
   * @param tool Tool name
   * @param input Input object (will be MessagePack-encoded and base64-serialized)
   * @returns Tool result as a string
   * @throws Error if plugin is unavailable (503, retryable), timeout, or plugin returns error
   */
  async invoke(pluginId: string, tool: string, input: unknown): Promise<string> {
    const tier = this.pluginTiers.get(pluginId) ?? "untrusted";
    const timeout = INVOKE_TIMEOUT[tier] ?? INVOKE_TIMEOUT.untrusted;

    const payload = encode({ pluginId, tool, input });
    const resp = await this.rpcWithTimeout(
      "plugin.invoke",
      { pluginId, tool, input: Buffer.from(payload).toString("base64") },
      timeout,
    );

    const data = resp as { result?: string; error?: string; unavailable?: boolean };
    if (data.unavailable) {
      throw Object.assign(new Error(`plugin ${pluginId} is draining`), { status: 503, retryable: true });
    }
    if (data.error) {
      throw new Error(data.error);
    }
    return data.result ?? "";
  }

  /**
   * RPC `plugin.dispatch` — Forward an event to plugins that subscribed to it.
   * @param event Event with type, optional pluginId, and generic payload
   */
  async dispatchEvent(event: PluginEvent): Promise<void> {
    await this.rpc("plugin.dispatch", event);
  }

  /**
   * RPC `health.check` — Check plugin host health status.
   * Never throws; always returns a HealthStatus object.
   * @returns Health status including connected flag, per-plugin state, and uptime in ms
   */
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

  /**
   * Whether the bridge is currently connected to the plugin host socket.
   * Check this before calling other methods to avoid "not connected" errors.
   */
  get connected(): boolean {
    return this.socket !== null && !this.socket.destroyed;
  }

  // ── Auto-registration ──────────────────────────────────────────────

  /**
   * Refresh plugin tools and auto-register them into the tool registry.
   * Fetches manifests to learn trust tiers, then fetches all tools and registers each.
   * Tools are registered with namespaced names: `plugin:<pluginId>:<toolName>`.
   * Unregisters stale tools from previous registrations before registering new ones.
   *
   * Called automatically on successful connect() if a tool registry was provided.
   * Can also be called manually to handle plugin hot-reload without reconnecting.
   *
   * @throws Error if RPC calls fail (manifests or tools not available)
   */
  async refreshTools(): Promise<void> {
    if (!this.toolRegistry) return;

    // Unregister stale tools
    this.unregisterAllTools();

    // Fetch manifests to get tier info
    const manifests = await this.listManifests();
    for (const m of manifests) {
      this.pluginTiers.set(m.id, m.trust_tier);
    }

    // Fetch all tools and register them
    const resp = await this.rpc("plugin.tools", {});
    const data = resp as { tools?: Array<{ plugin_id: string; tool: ToolInfo }> };

    for (const entry of data.tools ?? []) {
      const toolName = `plugin:${entry.plugin_id}:${entry.tool.name}`;
      this.toolRegistry.register({
        name: toolName,
        description: entry.tool.description,
        inputSchema: entry.tool.input_schema,
        execute: (input) => this.invoke(entry.plugin_id, entry.tool.name, input),
      });
      this.registeredTools.add(toolName);
    }
  }

  /** Unregister all currently registered plugin tools from the registry. */
  private unregisterAllTools(): void {
    if (!this.toolRegistry) return;
    for (const name of this.registeredTools) {
      this.toolRegistry.unregister(name);
    }
    this.registeredTools.clear();
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

  private handleData(chunk: string): void {
    this.buffer += chunk;
    let newlineIdx: number;
    while ((newlineIdx = this.buffer.indexOf("\n")) !== -1) {
      const line = this.buffer.slice(0, newlineIdx);
      this.buffer = this.buffer.slice(newlineIdx + 1);

      if (!line.trim()) continue;

      try {
        const resp: JsonRpcResponse = JSON.parse(line);
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

        // Auto-register tools on connect
        this.refreshTools().catch((err) =>
          console.warn("[plugin-bridge] tool refresh failed:", err.message),
        );

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
