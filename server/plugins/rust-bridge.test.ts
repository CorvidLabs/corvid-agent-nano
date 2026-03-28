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

    // Default handlers
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

  test("listTools returns tool info", async () => {
    await bridge.connect(host.socketPath);
    const tools = await bridge.listTools();
    expect(tools).toHaveLength(1);
    expect(tools[0].name).toBe("set_threshold");
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
});

describe("registerPluginRoutes", () => {
  // Import here to avoid issues if the module has side effects
  test("module exports registerPluginRoutes", async () => {
    const mod = await import("../routes/plugins");
    expect(typeof mod.registerPluginRoutes).toBe("function");
  });
});
