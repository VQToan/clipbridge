export interface Envelope {
  v: number;
  id: string;
  group_id: string;
  sender: string;
  created_at: number;
  nonce: string;
  ciphertext: string;
}

export type ClientFrame = {
  type: "publish";
  envelope: Envelope;
};

export type ServerFrame =
  | { type: "deliver"; envelope: Envelope }
  | { type: "ack"; id: string }
  | { type: "error"; code: string; message: string; id?: string };

export interface SessionAttachment {
  groupId: string;
  deviceId: string;
  connectedAt: number;
  tokens: number;
  lastTokenRefill: number;
}

export interface Env {
  RELAY_GROUP: DurableObjectNamespace;
}
