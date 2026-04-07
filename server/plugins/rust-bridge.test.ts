/**
 * Tests for the Plugin Bridge.
 *
 * Uses a mock Unix socket server to test JSON-RPC communication,
 * auto-registration, reconnection, and error handling.
 */

import { describe, test, expect, beforeEach, afterEach } from "bun:test";
import { createServer, type Server, type Socket as NetSocket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { unlinkSync, existsSync } from "node:fs";
import { PluginBridge, type ToolInfo } from "./rust-bridge";

// ── Mock server ────────────────────────────────────────────────────────

class MockPluginHost {
  server: Server | null = null;
  clients: NetSocket[] = [];
  socketPath: string;
  handlers: Record<string, (params: unknown) => unknown> = {};

  constructor() {
    this.socketPath = join(tmpdir(), `test-plugin-${Date.now()}-${Math.random().toString(36).slice(2)}.sock`);
  }

  async start(): Promise<void> {
    return new Promise((resolve) => {
      this.server = createServer((client) => {
        this.clients.push(client);
        let buffer = "";

        client.setEncoding("utf-8");
        client.on("data", (chunk) => {
          buffer += chunk;
          let idx: number;
          while ((idx = buffer.indexOf("\n")) !== -1) {
            const line = buffer.slice(0, idx);
            buffer = buffer.slice(idx + 1);
            this.handleRequest(client, line);
          }
        });
      });

      this.server!.listen(this.socketPath, () => resolve());
    });
  }

  private handleRequest(client: NetSocket, line: string): void {
    let id: unknown = null;
    try {
      const req = JSON.parse(line);
      id = req.id;
      const handler = this.handlers[req.method];
      if (handler) {
        const result = handler(req.params);
        client.write(JSON.stringify({ result, id: req.id }) + "\n");
      } else {
        client.write(
          JSON.stringify({ error: `unknown method: ${req.method}`, id: req.id }) + "\n",
        );
      }
    } catch (err) {
      client.write(JSON.stringify({ error: (err as Error).message, id }) + "\n");
    }
  }

  /** Broadcast a server-pushed notification to all connected clients. */
  broadcast(message: unknown): void {
    const line = JSON.stringify(message) + "\n";
    for (const client of this.clients) {
      client.write(line);
    }
  }

  async stop(): Promise<void> {
    for (const c of this.clients) c.destroy();
    this.clients = [];
    return new Promise((resolve) => {
      if (this.server) {
        this.server.close(() => resolve());
      } else {
        resolve();
      }
      if (existsSync(this.socketPath)) {
        try { unlinkSync(this.socketPath); } catch { /* noop */ }
      }
    });
  }
}

// ── Mock tool registry ─────────────────────────────────────────────────

class MockToolRegistry {
  tools = new Map<string, { name: string; description: string }>();

  register(entry: { name: string; description: string; inputSchema: Record<string, unknown>; execute: (input: unknown) => Promise<string> }) {
    this.tools.set(entry.name, { name: entry.name, description: entry.description });
  }

  unregister(name: string) {
    this.tools.delete(name);
  }
}

// ── Tests ──────────────────────────────────────────────────────────────

describe("PluginBridge", () => {
  let host: MockPluginHost;
  let bridge: PluginBridge;
  let registry: MockToolRegistry;

  beforeEach(async () => {
    host = new MockPluginHost();
    registry = new MockToolRegistry();

    // Default handlers — wire format uses snake_case (Rust serialization)
    host.handlers["plugin.list"] = () => ({
      plugins: [
        {
          id: "corvid-algo-oracle",
          version: "1.0.0",
          author: "corvidlabs",
          description: "Oracle plugin",
          capabilities: ["http", "algo"],
          trust_tier: "trusted",
          tools: [],
        },
      ],
    });

    host.handlers["plugin.tools"] = () => ({
      tools: [
        {
          plugin_id: "corvid-algo-oracle",
          tool: {
            name: "set_threshold",
            description: "Set the oracle threshold",
            input_schema: { type: "object", properties: { value: { type: "number" } } },
          },
        },
      ],
    });

    host.handlers["health.check"] = () => ({
      plugins: { "corvid-algo-oracle": "active" },
      uptime_ms: 12345,
    });

    await host.start();
    bridge = new PluginBridge({ toolRegistry: registry });
  });

  afterEach(async () => {
    await bridge.disconnect();
    await host.stop();
  });

  test("connects to Unix socket", async () => {
    await bridge.connect(host.socketPath);
    expect(bridge.connected).toBe(true);
  });

  test("listManifests returns plugin list", async () => {
    await bridge.connect(host.socketPath);
    const manifests = await bridge.listManifests();
    expect(manifests).toHaveLength(1);
    expect(manifests[0].id).toBe("corvid-algo-oracle");
    expect(manifests[0].version).toBe("1.0.0");
  });

  test("listManifests maps trust_tier to trustTier (camelCase)", async () => {
    await bridge.connect(host.socketPath);
    const manifests = await bridge.listManifests();
    expect(manifests[0].trustTier).toBe("trusted");
    // Wire field should not be present on public type
    expect((manifests[0] as unknown as Record<string, unknown>).trust_tier).toBeUndefined();
  });

  test("listTools returns tool info", async () => {
    await bridge.connect(host.socketPath);
    const tools = await bridge.listTools();
    expect(tools).toHaveLength(1);
    expect(tools[0].name).toBe("set_threshold");
  });

  test("listTools maps input_schema to inputSchema (camelCase)", async () => {
    await bridge.connect(host.socketPath);
    const tools = await bridge.listTools();
    expect(tools[0].inputSchema).toEqual({ type: "object", properties: { value: { type: "number" } } });
    // Wire field should not be present on public type
    expect((tools[0] as unknown as Record<string, unknown>).input_schema).toBeUndefined();
  });

  test("healthCheck returns status", async () => {
    await bridge.connect(host.socketPath);
    const health = await bridge.healthCheck();
    expect(health.connected).toBe(true);
    expect(health.uptimeMs).toBe(12345);
    expect(health.plugins["corvid-algo-oracle"]).toBe("active");
  });

  test("healthCheck returns disconnected when not connected", async () => {
    const unconnected = new PluginBridge();
    const health = await unconnected.healthCheck();
    expect(health.connected).toBe(false);
  });

  test("auto-registers tools on connect", async () => {
    await bridge.connect(host.socketPath);
    // Give auto-registration a moment to complete
    await new Promise((r) => setTimeout(r, 100));
    expect(registry.tools.has("plugin:corvid-algo-oracle:set_threshold")).toBe(true);
  });

  test("unregisters tools on disconnect", async () => {
    await bridge.connect(host.socketPath);
    await new Promise((r) => setTimeout(r, 100));
    expect(registry.tools.size).toBeGreaterThan(0);

    await bridge.disconnect();
    expect(registry.tools.size).toBe(0);
  });

  test("rpc error rejects promise", async () => {
    host.handlers["plugin.tools"] = () => {
      throw new Error("boom");
    };
    await bridge.connect(host.socketPath);
    await expect(bridge.listTools()).rejects.toThrow();
  });

  test("invoke sends tool invocation", async () => {
    host.handlers["plugin.invoke"] = (params: unknown) => {
      const p = params as { pluginId: string; tool: string };
      return { result: `invoked ${p.pluginId}:${p.tool}` };
    };

    await bridge.connect(host.socketPath);
    const result = await bridge.invoke("corvid-algo-oracle", "set_threshold", { value: 42 });
    expect(result).toBe("invoked corvid-algo-oracle:set_threshold");
  });

  test("invoke rejects on draining plugin", async () => {
    host.handlers["plugin.invoke"] = () => ({ unavailable: true });

    await bridge.connect(host.socketPath);
    try {
      await bridge.invoke("corvid-algo-oracle", "set_threshold", {});
      expect(true).toBe(false); // should not reach
    } catch (err) {
      expect((err as Error).message).toContain("draining");
      expect((err as Error & { status: number }).status).toBe(503);
    }
  });

  test("invoke surfaces string error from host", async () => {
    host.handlers["plugin.invoke"] = () => ({ error: "threshold out of range" });

    await bridge.connect(host.socketPath);
    await expect(bridge.invoke("corvid-algo-oracle", "set_threshold", {}))
      .rejects.toThrow("threshold out of range");
  });

  test("invoke with non-string error falls back to generic message", async () => {
    host.handlers["plugin.invoke"] = () => ({ error: { code: 42 } });

    await bridge.connect(host.socketPath);
    await expect(bridge.invoke("corvid-algo-oracle", "set_threshold", {}))
      .rejects.toThrow("plugin error (corvid-algo-oracle:set_threshold)");
  });

  test("oversized receive terminates socket", async () => {
    await bridge.connect(host.socketPath);

    // Send >1 MiB without a newline to trigger the buffer limit
    const oversized = "x".repeat(1_100_000);
    for (const client of host.clients) {
      client.write(oversized);
    }

    // Give the bridge a moment to process and close
    await new Promise((r) => setTimeout(r, 100));
    expect(bridge.connected).toBe(false);
  });

  test("disconnect rejects pending requests", async () => {
    // Set up a handler that never responds
    host.handlers["plugin.list"] = () => {
      return new Promise(() => {}); // never resolves
    };

    await bridge.connect(host.socketPath);
    const pending = bridge.listManifests();
    await bridge.disconnect();
    await expect(pending).rejects.toThrow("bridge disconnected");
  });

  test("tool names are namespaced plugin:id:name", async () => {
    await bridge.connect(host.socketPath);
    await new Promise((r) => setTimeout(r, 100));

    const names = [...registry.tools.keys()];
    for (const name of names) {
      expect(name).toMatch(/^plugin:[^:]+:[^:]+$/);
    }
  });

  test("plugin.tools_registered notification registers tools", async () => {
    // Start with no tools from initial fetch
    host.handlers["plugin.tools"] = () => ({ tools: [] });

    await bridge.connect(host.socketPath);
    await new Promise((r) => setTimeout(r, 100));
    expect(registry.tools.size).toBe(0);

    // Server pushes a tools_registered notification
    host.broadcast({
      event: "plugin.tools_registered",
      pluginId: "corvid-algo-oracle",
      trust_tier: "trusted",
      tools: [
        {
          name: "set_threshold",
          description: "Set the oracle threshold",
          input_schema: { type: "object" },
        },
      ],
    });

    await new Promise((r) => setTimeout(r, 50));
    expect(registry.tools.has("plugin:corvid-algo-oracle:set_threshold")).toBe(true);
  });

  test("plugin.tools_registered notification replaces only that plugin's tools", async () => {
    // Register two plugins up front
    host.handlers["plugin.list"] = () => ({
      plugins: [
        { id: "plugin-a", version: "1.0.0", author: "x", description: "a", capabilities: [], trust_tier: "trusted", tools: [] },
        { id: "plugin-b", version: "1.0.0", author: "x", description: "b", capabilities: [], trust_tier: "trusted", tools: [] },
      ],
    });
    host.handlers["plugin.tools"] = () => ({
      tools: [
        { plugin_id: "plugin-a", tool: { name: "tool-a1", description: "a1", input_schema: {} } },
        { plugin_id: "plugin-b", tool: { name: "tool-b1", description: "b1", input_schema: {} } },
      ],
    });

    await bridge.connect(host.socketPath);
    await new Promise((r) => setTimeout(r, 100));

    expect(registry.tools.has("plugin:plugin-a:tool-a1")).toBe(true);
    expect(registry.tools.has("plugin:plugin-b:tool-b1")).toBe(true);

    // Hot-reload plugin-a with new tools
    host.broadcast({
      event: "plugin.tools_registered",
      pluginId: "plugin-a",
      trust_tier: "trusted",
      tools: [
        { name: "tool-a2", description: "a2 new", input_schema: {} },
      ],
    });

    await new Promise((r) => setTimeout(r, 50));

    // plugin-a tools replaced
    expect(registry.tools.has("plugin:plugin-a:tool-a1")).toBe(false);
    expect(registry.tools.has("plugin:plugin-a:tool-a2")).toBe(true);
    // plugin-b untouched
    expect(registry.tools.has("plugin:plugin-b:tool-b1")).toBe(true);
  });

  test("plugin.tools_registered notification without pluginId is ignored", async () => {
    host.handlers["plugin.tools"] = () => ({ tools: [] });

    await bridge.connect(host.socketPath);
    await new Promise((r) => setTimeout(r, 100));

    // Malformed notification — no pluginId
    host.broadcast({ event: "plugin.tools_registered", tools: [] });
    await new Promise((r) => setTimeout(r, 50));

    // No crash, no tools registered
    expect(registry.tools.size).toBe(0);
  });
});

describe("registerPluginRoutes", () => {
  // Import here to avoid issues if the module has side effects
  test("module exports registerPluginRoutes", async () => {
    const mod = await import("../routes/plugins");
    expect(typeof mod.registerPluginRoutes).toBe("function");
  });
});

// ── Security tests ─────────────────────────────────────────────────────

describe("PluginBridge security", () => {
  let host: MockPluginHost;
  let bridge: PluginBridge;

  beforeEach(async () => {
    host = new MockPluginHost();
    await host.start();
    bridge = new PluginBridge();
  });

  afterEach(async () => {
    await bridge.disconnect();
    await host.stop();
  });

  test("buffer overflow closes socket", async () => {
    // Lower the limit to 1 KiB so the test is deterministic without sending MiBs.
    // Cast through unknown because the property is private static readonly at TS level
    // but a regular JS property at runtime (not inlined).
    const origLimit = (PluginBridge as unknown as { MAX_BUFFER_BYTES: number }).MAX_BUFFER_BYTES;
    (PluginBridge as unknown as { MAX_BUFFER_BYTES: number }).MAX_BUFFER_BYTES = 1024;

    await bridge.connect(host.socketPath);
    expect(bridge.connected).toBe(true);

    // Send 2 KiB without a newline — exceeds the patched 1 KiB limit.
    host.clients[0].write("x".repeat(2048));

    // Poll until disconnected (up to 1 s). The reconnect timer is 500 ms, so
    // there is a ~500 ms window where connected === false that we must catch.
    let disconnected = false;
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 50));
      if (!bridge.connected) { disconnected = true; break; }
    }

    (PluginBridge as unknown as { MAX_BUFFER_BYTES: number }).MAX_BUFFER_BYTES = origLimit;
    expect(disconnected).toBe(true);
  });

  test("bridge handles normal responses after large-but-valid payloads", async () => {
    // A response followed by a newline (even if large) should be processed normally.
    host.handlers["plugin.list"] = () => ({ plugins: [] });

    await bridge.connect(host.socketPath);
    const manifests = await bridge.listManifests();
    expect(manifests).toHaveLength(0);
  });
});

describe("Plugin route input validation", () => {
  /** Build a minimal mock bridge that always returns connected. */
  function makeMockBridge() {
    return {
      connected: true,
      async listManifests() { return []; },
      async listTools() { return []; },
      async invoke(pluginId: string, tool: string, _input: unknown) {
        return `ok:${pluginId}:${tool}`;
      },
    };
  }

  function makeRouter() {
    const routes: Array<{ method: string; path: string; handler: (ctx: { params: Record<string, string>; json(): Promise<unknown> }) => Promise<Response> | Response }> = [];
    return {
      routes,
      get(path: string, handler: (ctx: { params: Record<string, string>; json(): Promise<unknown> }) => Promise<Response> | Response) {
        routes.push({ method: "GET", path, handler });
      },
      post(path: string, handler: (ctx: { params: Record<string, string>; json(): Promise<unknown> }) => Promise<Response> | Response) {
        routes.push({ method: "POST", path, handler });
      },
      async call(params: Record<string, string>, body: unknown = {}) {
        const route = routes.find((r) => r.method === "POST");
        if (!route) throw new Error("no POST route");
        return route.handler({ params, json: async () => body });
      },
    };
  }

  test("rejects invalid plugin id with 400", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const res = await router.call({ id: "../../../etc/passwd", tool: "read" });
    expect(res.status).toBe(400);
    const body = await res.json() as { error: string };
    expect(body.error).toBe("invalid plugin id");
  });

  test("rejects id with uppercase letters", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const res = await router.call({ id: "MyPlugin", tool: "do_thing" });
    expect(res.status).toBe(400);
    const body = await res.json() as { error: string };
    expect(body.error).toBe("invalid plugin id");
  });

  test("rejects invalid tool name with 400", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const res = await router.call({ id: "my-plugin", tool: "../../etc/shadow" });
    expect(res.status).toBe(400);
    const body = await res.json() as { error: string };
    expect(body.error).toBe("invalid tool name");
  });

  test("accepts valid plugin id and tool name", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const res = await router.call({ id: "corvid-algo-oracle", tool: "set_threshold" });
    expect(res.status).toBe(200);
    const body = await res.json() as { result: string };
    expect(body.result).toBe("ok:corvid-algo-oracle:set_threshold");
  });

  test("rejects plugin id starting with digit", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const res = await router.call({ id: "1bad-plugin", tool: "run" });
    expect(res.status).toBe(400);
  });

  test("rejects plugin id longer than 50 chars", async () => {
    const { registerPluginRoutes } = await import("../routes/plugins");
    const router = makeRouter();
    registerPluginRoutes(router as never, makeMockBridge() as never);

    const longId = "a" + "b".repeat(50); // 51 chars
    const res = await router.call({ id: longId, tool: "run" });
    expect(res.status).toBe(400);
  });
});
