/**
 * Plugin REST routes — `/api/plugins` endpoints.
 *
 * Thin layer over PluginBridge. Registers with the server router
 * and delegates all work to the bridge.
 */

import type { PluginBridge, PluginManifest, ToolInfo } from "../plugins/rust-bridge";

// ── Validation ─────────────────────────────────────────────────────────

/** Matches the same regex enforced by the Rust loader: ^[a-z][a-z0-9-]{0,49}$ */
const PLUGIN_ID_RE = /^[a-z][a-z0-9-]{0,49}$/;

/** Tool names from plugin manifests are lowercase with underscores. */
const TOOL_NAME_RE = /^[a-z_][a-z0-9_]{0,63}$/;

// ── Types ──────────────────────────────────────────────────────────────

interface Router {
  get(path: string, handler: RouteHandler): void;
  post(path: string, handler: RouteHandler): void;
}

interface RouteContext {
  params: Record<string, string>;
  json(): Promise<unknown>;
}

type RouteHandler = (ctx: RouteContext) => Promise<Response> | Response;

interface PluginListItem extends PluginManifest {
  tools: ToolInfo[];
}

// ── Route registration ─────────────────────────────────────────────────

export function registerPluginRoutes(router: Router, bridge: PluginBridge): void {
  /**
   * GET /api/plugins — list all plugins with their tools.
   */
  router.get("/api/plugins", async () => {
    if (!bridge.connected) {
      return Response.json(
        { error: "plugin host not connected", plugins: [] },
        { status: 503 },
      );
    }

    try {
      const manifests = await bridge.listManifests();
      const plugins: PluginListItem[] = [];

      for (const manifest of manifests) {
        const tools = await bridge.listTools(manifest.id);
        plugins.push({ ...manifest, tools });
      }

      return Response.json({ plugins });
    } catch (err) {
      return Response.json(
        { error: (err as Error).message, plugins: [] },
        { status: 502 },
      );
    }
  });

  /**
   * POST /api/plugins/:id/invoke/:tool — invoke a specific plugin tool.
   *
   * Request body is passed as-is to the plugin tool.
   * Returns `{ result: string }` on success.
   */
  router.post("/api/plugins/:id/invoke/:tool", async (ctx) => {
    if (!bridge.connected) {
      return Response.json(
        { error: "plugin host not connected" },
        { status: 503 },
      );
    }

    const { id, tool } = ctx.params;
    if (!id || !tool) {
      return Response.json(
        { error: "missing plugin id or tool name" },
        { status: 400 },
      );
    }

    if (!PLUGIN_ID_RE.test(id) || !TOOL_NAME_RE.test(tool)) {
      return Response.json(
        { error: "invalid plugin id or tool name format" },
        { status: 400 },
      );
    }

    let input: unknown;
    try {
      input = await ctx.json();
    } catch {
      return Response.json(
        { error: "invalid request body: expected JSON" },
        { status: 400 },
      );
    }

    try {
      const result = await bridge.invoke(id, tool, input);
      return Response.json({ result });
    } catch (err) {
      const error = err as Error & { status?: number; retryable?: boolean };
      const status = error.status ?? 500;
      return Response.json(
        { error: error.message, retryable: error.retryable ?? false },
        { status },
      );
    }
  });
}
