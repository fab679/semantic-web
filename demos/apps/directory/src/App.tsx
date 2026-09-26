// Harborlight Directory (:5173 → service :8484)
//
// The Directory app speaks the shared intersection (foaf:name, rdf:type)
// plus its own terms (knows, worksFor, foundingDate). Its detail view
// reads EVERY predicate on a subject — including facts written by the
// Board app — because the graph is shared. Badges show who wrote what.

import { useEffect, useMemo, useState } from "react";
import {
  createClient,
  namesOf,
  ownerBadgeLabel,
  recordsOf,
  resolverFromManifest,
  type FactRecord,
  type Owner,
} from "@demos/shared";
import { Badge } from "@demos/shared";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@demos/shared";
import { Skeleton } from "@demos/shared";
import { Users } from "lucide-react";

const SELF = (import.meta.env.VITE_SEMWEB_URL as string | undefined) ?? "http://localhost:8484";
const BOARD = (import.meta.env.VITE_BOARD_URL as string | undefined) ?? "http://localhost:8487";

const TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PERSON = "http://xmlns.com/foaf/0.1/Person";
const NAME = "http://xmlns.com/foaf/0.1/name";

const self = createClient(SELF);
const board = createClient(BOARD);

export default function App() {
  const [people, setPeople] = useState<string[]>([]);
  const [names, setNames] = useState<Map<string, string>>(new Map());
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [types, nameLines, manifest] = await Promise.all([
          self.fetchFragments({ predicate: TYPE, object: PERSON, limit: 100 }),
          self.fetchFragments({ predicate: NAME, limit: 200 }),
          self.fetchManifest(),
        ]);
        if (cancelled) return;
        const resolve = resolverFromManifest(manifest.predicates);
        setPeople(types.lines.map((l) => l["@id"] as string).filter(Boolean));
        setNames(namesOf(nameLines.lines, resolve));
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="min-h-screen">
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex max-w-5xl items-center justify-between gap-3 px-6 py-3">
          <h1 className="flex items-center gap-2 text-lg font-semibold tracking-tight">
            <Users className="size-4 text-primary" aria-hidden="true" />
            Harborlight Directory
          </h1>
          <a href={SELF} className="font-mono text-xs text-muted-foreground underline-offset-4 hover:underline">
            {SELF}
          </a>
        </div>
      </header>
      <main className="mx-auto max-w-5xl px-6 py-8">
        {error && <p className="mb-6 font-mono text-sm text-destructive">{error}</p>}
        <div className="grid gap-8 lg:grid-cols-[300px_1fr]">
          <section>
            <h2 className="mb-3 text-sm font-medium text-muted-foreground">Residents</h2>
            {people.length === 0 ? (
              <Skeleton className="h-40 w-full" />
            ) : (
              <ul>
                {people.map((p) => (
                  <li key={p}>
                    <button
                      type="button"
                      onClick={() => setSelected(p)}
                      className={`w-full border-b border-border px-2 py-2.5 text-left focus-visible:outline-2 focus-visible:outline-ring ${
                        selected === p ? "bg-secondary" : "hover:bg-secondary/50"
                      }`}
                    >
                      <span className="font-medium">{names.get(p) ?? p}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <section>
            {selected ? (
              <PersonDetail uri={selected} names={names} />
            ) : (
              <p className="text-sm text-muted-foreground">
                Pick a resident. Their record shows facts from every app —
                this page reads the shared graph, not its own store alone.
              </p>
            )}
          </section>
        </div>
      </main>
    </div>
  );
}

const OWNER_TINT: Record<Owner, string> = {
  shared: "text-muted-foreground",
  directory: "text-primary",
  market: "text-foreground",
  board: "text-signal",
  other: "text-muted-foreground",
};

function PersonDetail({ uri, names }: { uri: string; names: Map<string, string> }) {
  const [records, setRecords] = useState<FactRecord[] | null>(null);
  const [mentions, setMentions] = useState<string[]>([]);
  const loading = records === null;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Own store: every predicate on this subject — whatever app wrote it.
        const [manifest, page] = await Promise.all([
          self.fetchManifest(),
          self.fetchFragments({ subject: uri, limit: 100 }),
        ]);
        // Cross-app: does the BOARD mention this person? One GET on the
        // board's service — no integration, no API key.
        const boardMentions = await board.fetchFragments({
          predicate: "http://harborlight.example/board#mentions",
          object: uri,
          limit: 20,
        });
        if (cancelled) return;
        const resolve = resolverFromManifest(manifest.predicates);
        setRecords(recordsOf(page.lines, resolve));
        setMentions(boardMentions.lines.map((l) => l["@id"] as string).filter(Boolean));
      } catch {
        // cross-service read is best-effort
        if (!cancelled) setRecords([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [uri]);

  const groups = useMemo(() => {
    const g = new Map<Owner, FactRecord[]>();
    for (const r of records ?? []) {
      const owner = ownerOfRecord(r.uri);
      if (!g.has(owner)) g.set(owner, []);
      g.get(owner)!.push(r);
    }
    return g;
  }, [records]);

  const label = (u: string) => names.get(u) ?? u;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-xl">{names.get(uri) ?? uri}</CardTitle>
        <CardDescription className="break-all font-mono text-xs">{uri}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        {loading ? (
          <Skeleton className="h-24 w-full" />
        ) : records && records.length === 0 && (
          <p className="text-sm text-muted-foreground">No facts reference this URI yet.</p>
        )}
        {[...groups.entries()].map(([owner, recs]) => (
          <section key={owner} className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
              <Badge variant={owner === "board" ? "outline" : "secondary"}>{ownerBadgeLabel(owner)}</Badge>
              <span className={`text-xs ${OWNER_TINT[owner]}`}>{recs.length} fact{recs.length === 1 ? "" : "s"}</span>
            </div>
            <div className="rounded-lg border border-border bg-background">
              {recs.map((r, i) => (
                <p key={i} className={`data-line px-4 py-2 ${i > 0 ? "border-t border-border" : ""}`}>
                  <span className="text-muted-foreground">{r.key}</span>{" "}
                  {looksLikeUri(r.value) && (names.has(r.value) || r.key === "knows" || r.key === "employer") ? (
                    <span className="text-primary">{label(r.value)}</span>
                  ) : (
                    r.value
                  )}
                </p>
              ))}
            </div>
          </section>
        ))}
        {mentions.length > 0 && (
          <section className="flex flex-col gap-2">
            <Badge variant="outline" className="w-fit border-signal/40 text-signal">
              mentioned by Board ({mentions.length} post{mentions.length === 1 ? "" : "s"})
            </Badge>
            <p className="text-sm text-muted-foreground">
              These posts live in the Board app's own store on another
              service. The Directory found them with one fragment query.
            </p>
          </section>
        )}
      </CardContent>
    </Card>
  );
}

// local owner check (predicate uri -> owner) without importing internal map
function ownerOfRecord(uri: string): Owner {
  if (uri === NAME || uri === TYPE) return "shared";
  if (uri.startsWith("http://harborlight.example/board#")) return "board";
  if (uri.startsWith("http://harborlight.example/market#")) return "market";
  if (uri.startsWith("http://xmlns.com/foaf") || uri.startsWith("http://schema.org")) return "directory";
  return "other";
}

function looksLikeUri(v: string): boolean {
  return v.startsWith("http://") || v.startsWith("https://");
}