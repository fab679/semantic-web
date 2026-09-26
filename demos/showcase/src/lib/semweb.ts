// Client for the semantic-web service. Every call maps to one HTTP
// surface documented in the infrastructure's reference.md — no wrapper
// SDK, that is the point being demonstrated.

export const SERVICE_URL: string =
  (import.meta.env.VITE_SEMWEB_URL as string | undefined) ?? "http://localhost:8484";

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(`${SERVICE_URL}${path}`);
  if (!res.ok) throw new Error(`${path} → ${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

export interface ManifestClass {
  uri: string;
  compact?: string;
  instances: number;
  description?: string | null;
  shapes?: { path: string; minCount: number; maxCount: number | null; datatype?: string | null }[] | null;
  exampleQuery?: string;
}

export interface ManifestPredicate {
  uri: string;
  compact?: string;
  triples: number;
  description?: string | null;
}

export interface Manifest {
  kind: string;
  schemaFingerprint: string;
  prefixes: Record<string, string>;
  classes: ManifestClass[];
  predicates: ManifestPredicate[];
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

export async function fetchManifest(): Promise<Manifest> {
  return getJson<Manifest>("/manifest");
}

export async function fetchDid(): Promise<DidDocument> {
  return getJson<DidDocument>("/.well-known/did.json");
}

export async function fetchHealth(): Promise<boolean> {
  try {
    const res = await fetch(`${SERVICE_URL}/health`);
    return res.ok;
  } catch {
    return false;
  }
}

/** Stream one fragment page. The service answers with one JSON-LD line
 * per fact, plus a trailing control line (@control: metadata) carrying
 * the cursor and the count estimate. */
export async function fetchFragments(params: {
  subject?: string;
  predicate?: string;
  object?: string;
  after?: string;
  limit?: number;
}): Promise<FragmentPage> {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v) q.set(k, String(v));
  const res = await fetch(`${SERVICE_URL}/fragments?${q}`);
  if (!res.ok) throw new Error(`/fragments → ${res.status}`);
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
}

export interface InsertResult {
  ok: boolean;
  duplicate?: boolean;
  eventId?: string;
  schemaChanged?: boolean;
  message: string;
}

export async function insertTriple(args: {
  subject: string;
  predicate: string;
  object: string;
  token: string;
}): Promise<InsertResult> {
  const res = await fetch(`${SERVICE_URL}/admin/insert`, {
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
  const body = (await res.json()) as {
    event_id?: string;
    duplicate?: boolean;
    schema_changed?: boolean;
  };
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
}

export type McpToolResult = { isError?: boolean; content: { type: string; text: string }[]; _truncated?: boolean };

export async function mcpCall(tool: string, args: Record<string, unknown>): Promise<McpToolResult> {
  const res = await fetch(`${SERVICE_URL}/mcp`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: tool, arguments: args },
    }),
  });
  if (!res.ok) throw new Error(`/mcp → ${res.status}`);
  const body = (await res.json()) as { result?: McpToolResult; error?: { message: string } };
  if (body.error) throw new Error(body.error.message);
  return body.result as McpToolResult;
}

/** Verdict shape returned by the verify_claim tool (see docs/trust.md). */
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

export async function verifyDocument(document: unknown): Promise<Verdict> {
  const result = await mcpCall("verify_claim", { credential: document });
  return JSON.parse(result.content[0].text) as Verdict;
}

export async function issueAttestation(args: {
  claim: Record<string, unknown>;
  id?: string;
  validUntil?: string;
  supersedes?: string;
}): Promise<Record<string, unknown>> {
  const result = await mcpCall("issue_attestation", {
    claim: args.claim,
    ...(args.id ? { id: args.id } : {}),
    ...(args.validUntil ? { valid_until: args.validUntil } : {}),
    ...(args.supersedes ? { supersedes: args.supersedes } : {}),
  });
  return JSON.parse(result.content[0].text) as Record<string, unknown>;
}

export interface SseEvent {
  topic?: string;
  eventId?: string;
  receivedAt: Date;
  error?: string;
  raw?: string;
}

/** Open the SSE change feed (GET /events). Payload per event is
 * {"topic": ..., "event_id": ...} — see hub/publish.rs. Returns a close
 * function. */
export function openEvents(
  topic: string,
  onEvent: (e: SseEvent) => void,
  onStatus: (open: boolean) => void,
): () => void {
  const url = topic ? `${SERVICE_URL}/events?topic=${encodeURIComponent(topic)}` : `${SERVICE_URL}/events`;
  const source = new EventSource(url);
  source.onopen = () => onStatus(true);
  source.onerror = () => onStatus(false);
  source.onmessage = (msg) => {
    try {
      const parsed = JSON.parse(msg.data) as { topic?: string; event_id?: string; error?: string };
      onEvent({
        topic: parsed.topic,
        eventId: parsed.event_id,
        error: parsed.error,
        receivedAt: new Date(),
      });
    } catch {
      onEvent({ receivedAt: new Date(), raw: msg.data });
    }
  };
  return () => source.close();
}