import { DurableObject } from "cloudflare:workers";
import type { Envelope, ServerFrame, SessionAttachment, Env } from "./types";

const MAX_CONNECTIONS_PER_GROUP = 32;
const MAX_MAILBOX_ENVELOPES = 50;
const MAILBOX_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours
const PUBLISH_BURST = 20;
const PUBLISH_RATE_PER_SEC = 5;

// ULID format check: 26 characters Crockford base32
const ULID_REGEX = /^[0-9A-HJ-KM-NP-Za-km-np-z]{26}$/;
const HEX64_REGEX = /^[0-9a-f]{64}$/;

export class RelayGroup extends DurableObject<Env> {
  private groupId: string = "";

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ctx.blockConcurrencyWhile(async () => {
      this.groupId = (await this.ctx.storage.get<string>("group_id")) || "";
    });
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);

    // Ephemeral pairing rendezvous: create
    if (url.pathname === "/v1/pair/create" && request.method === "POST") {
      try {
        const body: any = await request.json();
        const code = body.code
          ? body.code.toString().replace(/[\s-]/g, "").toUpperCase()
          : String(Math.floor(100000 + Math.random() * 900000));

        if (!/^[0-9A-Z]{6,8}$/.test(code)) {
          return new Response(JSON.stringify({ error: "Code must be 6 to 8 alphanumeric characters" }), {
            status: 400,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
          });
        }
        if (!body.group_key || body.group_key.length !== 43) {
          return new Response(JSON.stringify({ error: "Invalid group_key" }), {
            status: 400,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
          });
        }

        const relayUrl = body.relay_url || "wss://clipbridge-relay.vqtoan1807.workers.dev";
        const expiresAt = Date.now() + 10 * 60 * 1000; // 10 minutes

        await this.ctx.storage.put(`pair:${code}`, {
          group_key: body.group_key,
          relay_url: relayUrl,
          expires_at: expiresAt,
        });

        return new Response(JSON.stringify({ status: "ok", code, expires_in_sec: 600 }), {
          status: 200,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
        });
      } catch (e: any) {
        return new Response(JSON.stringify({ error: e.message || "Failed to create pairing" }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
        });
      }
    }

    // Ephemeral pairing rendezvous: claim
    if (url.pathname === "/v1/pair/claim" && request.method === "POST") {
      try {
        const body: any = await request.json();
        const code = (body.code || "").toString().replace(/[\s-]/g, "").toUpperCase();
        const pairData = await this.ctx.storage.get<{ group_key: string; relay_url: string; expires_at: number }>(`pair:${code}`);

        if (!pairData || Date.now() > pairData.expires_at) {
          if (pairData) await this.ctx.storage.delete(`pair:${code}`);
          return new Response(JSON.stringify({ error: "Invalid or expired pairing code" }), {
            status: 404,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
          });
        }

        // Single-use: delete after claim
        await this.ctx.storage.delete(`pair:${code}`);

        return new Response(
          JSON.stringify({
            status: "ok",
            group_key: pairData.group_key,
            relay_url: pairData.relay_url,
          }),
          {
            status: 200,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
          }
        );
      } catch (e: any) {
        return new Response(JSON.stringify({ error: e.message || "Failed to claim pairing" }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" },
        });
      }
    }

    // Only /v1/ws is handled inside the group Durable Object
    if (url.pathname !== "/v1/ws") {
      return new Response("Not found", { status: 404 });
    }

    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("Expected WebSocket upgrade", { status: 426 });
    }

    const groupId = request.headers.get("X-ClipBridge-Group") || "";
    const deviceId = request.headers.get("X-ClipBridge-Device") || "";
    const authHeader = request.headers.get("Authorization") || "";
    const token = authHeader.startsWith("Bearer ") ? authHeader.slice(7) : "";

    this.groupId = groupId;
    await this.ctx.storage.put("group_id", groupId);

    // 1. Validate headers
    if (!HEX64_REGEX.test(groupId)) {
      return new Response("Invalid group ID format", { status: 400 });
    }
    if (!deviceId || deviceId.length > 64) {
      return new Response("Invalid device ID", { status: 400 });
    }
    if (!token || token.length !== 43) {
      return new Response("Invalid token", { status: 401 });
    }

    const since = url.searchParams.get("since");
    if (since && !ULID_REGEX.test(since)) {
      return new Response("since must be a valid ULID", { status: 400 });
    }

    // 2. Authenticate token via TOFU (Trust On First Use)
    const tokenHashHex = await this.hashToken(token);
    const storedHash = await this.ctx.storage.get<string>("auth_token_hash");

    if (storedHash === undefined) {
      // First connection registers group
      await this.ctx.storage.put("auth_token_hash", tokenHashHex);
    } else if (storedHash !== tokenHashHex) {
      return new Response("Unauthorized", { status: 401 });
    }

    // 3. Connection limit check
    const currentSockets = this.ctx.getWebSockets();
    if (currentSockets.length >= MAX_CONNECTIONS_PER_GROUP) {
      return new Response("Group connection limit exceeded", { status: 429 });
    }

    // 4. Accept WebSocket using Cloudflare Hibernation API
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    const attachment: SessionAttachment = {
      groupId,
      deviceId,
      connectedAt: Date.now(),
      tokens: PUBLISH_BURST,
      lastTokenRefill: Date.now(),
    };

    this.ctx.acceptWebSocket(server, [deviceId]);
    server.serializeAttachment(attachment);

    // 5. Replay backlog to newly connected client
    const mailbox = await this.getPrunedMailbox();
    for (const env of mailbox) {
      // Deliver if newer than `since` and not sent by this device
      if (env.sender !== deviceId) {
        if (!since || env.id > since) {
          const deliverFrame: ServerFrame = { type: "deliver", envelope: env };
          server.send(JSON.stringify(deliverFrame));
        }
      }
    }

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    if (typeof message !== "string") {
      this.sendError(ws, "invalid_frame", "only text frames are supported");
      return;
    }

    let parsed: any;
    try {
      parsed = JSON.parse(message);
    } catch {
      this.sendError(ws, "invalid_frame", "malformed JSON");
      return;
    }

    if (!parsed || parsed.type !== "publish" || !parsed.envelope) {
      this.sendError(ws, "invalid_frame", "expected publish frame with envelope");
      return;
    }

    const env: Envelope = parsed.envelope;
    const attachment = ws.deserializeAttachment() as SessionAttachment | null;
    const sender = attachment?.deviceId || env.sender;
    const expectedGroupId = attachment?.groupId || this.groupId || (await this.ctx.storage.get<string>("group_id"));

    // Validate Envelope
    if (env.v !== 1) {
      this.sendError(ws, "invalid_envelope", "unsupported envelope version", env.id);
      return;
    }
    if (!env.id || !ULID_REGEX.test(env.id)) {
      this.sendError(ws, "invalid_envelope", "envelope id must be a valid ULID", env.id);
      return;
    }
    if (env.sender !== sender) {
      this.sendError(ws, "invalid_envelope", "sender mismatch", env.id);
      return;
    }
    if (!env.group_id || (expectedGroupId && env.group_id !== expectedGroupId)) {
      this.sendError(ws, "invalid_envelope", "group_id mismatch", env.id);
      return;
    }
    if (!env.nonce || !env.ciphertext || typeof env.created_at !== "number") {
      this.sendError(ws, "invalid_envelope", "missing required fields", env.id);
      return;
    }

    // Rate Limiting Check
    if (attachment) {
      const now = Date.now();
      const elapsedSec = (now - attachment.lastTokenRefill) / 1000;
      attachment.tokens = Math.min(PUBLISH_BURST, attachment.tokens + elapsedSec * PUBLISH_RATE_PER_SEC);
      attachment.lastTokenRefill = now;

      if (attachment.tokens < 1) {
        this.sendError(ws, "rate_limited", "publish rate exceeded", env.id);
        ws.serializeAttachment(attachment);
        return;
      }
      attachment.tokens -= 1;
      ws.serializeAttachment(attachment);
    }

    // Save to Mailbox
    const mailbox = await this.getPrunedMailbox();
    mailbox.push(env);
    if (mailbox.length > MAX_MAILBOX_ENVELOPES) {
      mailbox.splice(0, mailbox.length - MAX_MAILBOX_ENVELOPES);
    }
    await this.ctx.storage.put("mailbox", mailbox);

    // Fan-out delivery to all OTHER connected sockets in this group
    const deliverFrame: ServerFrame = { type: "deliver", envelope: env };
    const deliverPayload = JSON.stringify(deliverFrame);

    for (const peer of this.ctx.getWebSockets()) {
      if (peer !== ws) {
        try {
          peer.send(deliverPayload);
        } catch (e) {
          console.warn("Failed to deliver frame to peer:", e);
        }
      }
    }

    // Send ACK back to publisher
    const ackFrame: ServerFrame = { type: "ack", id: env.id };
    ws.send(JSON.stringify(ackFrame));
  }

  async webSocketClose(ws: WebSocket, code: number, reason: string, wasClean: boolean): Promise<void> {
    try {
      ws.close(code, reason);
    } catch {}
  }

  async webSocketError(ws: WebSocket, error: unknown): Promise<void> {
    console.error("Relay WebSocket error:", error);
    try {
      ws.close(1011, "internal error");
    } catch {}
  }

  private sendError(ws: WebSocket, code: string, message: string, id?: string): void {
    const errorFrame: ServerFrame = { type: "error", code, message, id };
    try {
      ws.send(JSON.stringify(errorFrame));
    } catch {}
  }

  private async getPrunedMailbox(): Promise<Envelope[]> {
    const raw = await this.ctx.storage.get<Envelope[]>("mailbox");
    const list = Array.isArray(raw) ? raw : [];
    const cutoff = Date.now() - MAILBOX_TTL_MS;
    return list.filter((e) => e.created_at >= cutoff);
  }

  private async hashToken(token: string): Promise<string> {
    const data = new TextEncoder().encode(token);
    const digest = await crypto.subtle.digest("SHA-256", data);
    return Array.from(new Uint8Array(digest))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }
}
