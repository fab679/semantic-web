// Client for the semantic-web service, as a factory: each app binds a
// client to its own service (and sibling services for cross-app reads).
// Every call maps to one HTTP surface from the infrastructure's
// reference.md — no wrapper SDK; that is the point being demonstrated.

const DEFAULT_BASE =
  (import.meta.env.VITE_SEMWEB_URL as string | undefined) ?? "http://localhost:8484";

export interface Manifest {
  kind: string;
  schemaFingerprint: string;
  prefixes: Record<string, string>;
  classes: { uri: string; compact?: string; instances: number; description?: string | null; shapes?: { path: string; minCount: number; maxCount: number | null; datatype?: string | null }[] | null; exampleQuery?: string }[];
  predicates: { uri: string; compact?: string; triples: number; description?: string | null }[];
  topics: string[];
  issuer?: string;
  trust?: { cryptosuite: string; verificationMethod: string; didDocument: string; verifyWith: string };
  proof?: { type: string; cryptosuite: string; verificationMethod: string; created: string; proofPurpose: string; proofValue: string };
}

export interface DidDocument {
  id: string;
  verificationMethod: { id: string; type: string; controller: string; publicKeyMultibase: string }[];
  trust?: { trustedIssuers: number; revokedCredentials: number };
}

export interface FragmentParams {
  subject?: string;
  predicate?: string;
  object?: string;
  after?: string;
  limit?: number;
}

export interface FragmentPage {
  lines: Record<string, unknown>[];
  after?: string;
  countEstimate: number;
  hasMore: boolean;
}

interface FragmentLine extends Record<string, unknown> {
  "@id"?: string;
  "@control"?: string;
  after?: string;
  count_estimate?: number;
}

export interface InsertArgs {
  subject: string;
  predicate: string;
  object: string;
  token: string;
}

export interface InsertResult {
  ok: boolean;
  duplicate?: boolean;
  eventId?: string;
  schemaChanged?: boolean;
  message: string;
}

export type McpToolResult = { isError?: boolean; content: { type: string; text: string }[]; _truncated?: boolean };

export interface Verdict {
  verified: boolean;
  gates: {
    shape: boolean;
    signature: boolean;
    issuerTrusted: boolean;
    temporal: boolean | null;
    revocation: boolean | null;
  };
  issuer: string | null;
  details: string[];
}

export interface AttestationArgs {
  claim: Record<string, unknown>;
  id?: string;
  validUntil?: string;
  supersedes?: string;
}

export interface SseEvent {
  topic?: string;
  eventId?: string;
  receivedAt: Date;
  error?: string;
  raw?: string;
}

export function createClient(base: string = DEFAULT_BASE): Client {
  const b = base.replace(/\/$/, "");

  async function getJson<T>(path: string): Promise<T> {
    const res = await fetch(`${b}${path}`);
    if (!res.ok) throw new Error(`${base}${path} → ${res.status} ${res.statusText}`);
    return res.json() as Promise<T>;
  }

  async function mcpCall(tool: string, args: Record<string, unknown>): Promise<McpToolResult> {
    const res = await fetch(`${b}/mcp`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name: tool, arguments: args },
      }),
    });
    if (!res.ok) throw new Error(`${base}/mcp → ${res.status}`);
    const body = (await res.json()) as { result?: McpToolResult; error?: { message: string } };
    if (body.error) throw new Error(body.error.message);
    return body.result as McpToolResult;
  }

  return {
    base: b,

    fetchManifest: () => getJson<Manifest>("/manifest"),

    fetchDid: () => getJson<DidDocument>("/.well-known/did.json"),

    fetchHealth: async () => {
      try {
        const res = await fetch(`${b}/health`);
        return res.ok;
      } catch {
        return false;
      }
    },

    /** One fragment page: one JSON-LD line per fact, plus a trailing
     * control line (@control: metadata) carrying cursor + count. */
    fetchFragments: async (params) => {
      const q = new URLSearchParams();
      for (const [k, v] of Object.entries(params)) if (v) q.set(k, String(v));
      const res = await fetch(`${b}/fragments?${q}`);
      if (!res.ok) throw new Error(`${base}/fragments → ${res.status}`);
      const text = await res.text();
      const lines: Record<string, unknown>[] = [];
      let after: string | undefined;
      let countEstimate = 0;
      for (const raw of text.split("\n")) {
        if (!raw.trim()) continue;
        const parsed = JSON.parse(raw) as FragmentLine;
        if (parsed["@control"] === "metadata") {
          after = parsed.after;
          countEstimate = parsed.count_estimate ?? countEstimate;
        } else {
          lines.push(parsed);
        }
      }
      return { lines, after, countEstimate, hasMore: after !== undefined };
    },

    insertTriple: async (args) => {
      const res = await fetch(`${b}/admin/insert`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(args.token ? { Authorization: `Bearer ${args.token}` } : {}),
        },
        body: JSON.stringify({ subject: args.subject, predicate: args.predicate, object: args.object }),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        return { ok: false, message: `${res.status} ${res.statusText}${text ? ` — ${text}` : ""}` };
      }
      const body = (await res.json()) as { event_id?: string; duplicate?: boolean; schema_changed?: boolean };
      return {
        ok: true,
        duplicate: body.duplicate,
        eventId: body.event_id,
        schemaChanged: body.schema_changed,
        message: body.duplicate
          ? "No change — the triple already exists, so no event was pushed."
          : body.schema_changed
            ? "Inserted, and the schema changed — a schema event was pushed too."
            : "Inserted.",
      };
    },

    mcpCall,

    verifyDocument: async (document) => {
      const result = await mcpCall("verify_claim", { credential: document });
      return JSON.parse(result.content[0].text) as Verdict;
    },

    issueAttestation: async (args) => {
      const result = await mcpCall("issue_attestation", {
        claim: args.claim,
        ...(args.id ? { id: args.id } : {}),
        ...(args.validUntil ? { valid_until: args.validUntil } : {}),
        ...(args.supersedes ? { supersedes: args.supersedes } : {}),
      });
      return JSON.parse(result.content[0].text) as Record<string, unknown>;
    },

    /** Open the SSE change feed (GET /events). Payload per event is
     * {"topic", "event_id"} — see hub/publish.rs. Returns a close fn. */
    openEvents: (topic, onEvent, onStatus) => {
      const url = topic ? `${b}/events?topic=${encodeURIComponent(topic)}` : `${b}/events`;
      const source = new EventSource(url);
      source.onopen = () => onStatus(true);
      source.onerror = () => onStatus(false);
      source.onmessage = (msg) => {
        try {
          const parsed = JSON.parse(msg.data) as { topic?: string; event_id?: string; error?: string };
          onEvent({ topic: parsed.topic, eventId: parsed.event_id, error: parsed.error, receivedAt: new Date() });
        } catch {
          onEvent({ receivedAt: new Date(), raw: msg.data });
        }
      };
      return () => source.close();
    },
  };
}

export interface Client {
  base: string;
  fetchManifest(): Promise<Manifest>;
  fetchDid(): Promise<DidDocument>;
  fetchHealth(): Promise<boolean>;
  fetchFragments(params: FragmentParams): Promise<FragmentPage>;
  insertTriple(args: InsertArgs): Promise<InsertResult>;
  mcpCall(tool: string, args: Record<string, unknown>): Promise<McpToolResult>;
  verifyDocument(document: unknown): Promise<Verdict>;
  issueAttestation(args: AttestationArgs): Promise<Record<string, unknown>>;
  openEvents(topic: string, onEvent: (e: SseEvent) => void, onStatus: (open: boolean) => void): () => void;
}

export { DEFAULT_BASE };