/**
 * Plugin REST routes — `/api/plugins` endpoints.
 *
 * Thin HTTP layer over PluginBridge. Registers GET/POST handlers with the server router
 * and delegates all work to the bridge.
 *
 * Routes:
 * - GET /api/plugins           — List all plugins with their tools
 * - POST /api/plugins/:id/invoke/:tool — Invoke a specific plugin tool
 *
 * Status codes:
 * - 200: Success
 * - 400: Missing or invalid parameters
 * - 500: Plugin error or internal error
 * - 502: Bad gateway (RPC error from plugin host)
 * - 503: Plugin host not connected or plugin draining
 */

import type { PluginBridge, PluginManifest, ToolInfo } from "../plugins/rust-bridge";

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
   * GET /api/plugins
   *
   * List all loaded plugins with their metadata and tools.
   * Returns 503 if the plugin host is not connected.
   * Returns 502 if the plugin host RPC fails.
   *
   * Response on success:
   * ```json
   * {
   *   "plugins": [
   *     {
   *       "id": "corvid-algo-oracle",
   *       "version": "1.0.0",
   *       "author": "CorvidLabs",
   *       "description": "Oracle for Algorand app state",
   *       "capabilities": ["algo", "http"],
   *       "trust_tier": "verified",
   *       "tools": [
   *         {
   *           "name": "set_threshold",
   *           "description": "Set oracle threshold",
   *           "input_schema": { "type": "object", ... }
   *         }
   *       ]
   *     }
   *   ]
   * }
   * ```
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
   * POST /api/plugins/:id/invoke/:tool
   *
   * Invoke a specific plugin tool with the provided input.
   * The request body (any JSON) is passed directly to the tool.
   * Respects plugin trust tier timeouts (trusted: 30s, verified: 5s, untrusted: 1s).
   *
   * Path parameters:
   * - :id   — Plugin ID (e.g., "corvid-algo-oracle")
   * - :tool — Tool name (e.g., "set_threshold")
   *
   * Request body: any JSON object or value to pass to the tool
   *
   * Response on success:
   * ```json
   * { "result": "..string output from tool.." }
   * ```
   *
   * Response on error:
   * ```json
   * { "error": "error message", "retryable": true/false }
   * ```
   *
   * Status codes:
   * - 200: Tool executed successfully
   * - 400: Missing or invalid plugin ID / tool name
   * - 500: Tool returned an error (non-retryable)
   * - 503: Plugin host not connected OR plugin is draining (retryable: true)
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

    let input: unknown;
    try {
      input = await ctx.json();
    } catch {
      input = {};
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
