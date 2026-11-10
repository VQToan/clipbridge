import type { Env } from "./types";
export { RelayGroup } from "./relay-group";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    // Healthcheck
    if (url.pathname === "/healthz") {
      return new Response("ok", {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      });
    }

    // Root status
    if (url.pathname === "/") {
      return new Response("ClipBridge Relay v1 (Cloudflare Workers + Durable Objects)", {
        status: 200,
        headers: { "Content-Type": "text/plain" },
      });
    }

    // WebSocket relay endpoint
    if (url.pathname === "/v1/ws") {
      const groupId = request.headers.get("X-ClipBridge-Group");
      if (!groupId || !/^[0-9a-f]{64}$/.test(groupId)) {
        return new Response("Missing or invalid X-ClipBridge-Group header (expected 64 lowercase hex)", {
          status: 400,
        });
      }

      // Route by Group ID to its dedicated Durable Object instance
      const doId = env.RELAY_GROUP.idFromName(groupId);
      const stub = env.RELAY_GROUP.get(doId);
      return stub.fetch(request);
    }

    return new Response("Not found", { status: 404 });
  },
};
