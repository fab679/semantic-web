// Harborlight Market (:5174 → service :8486)
//
// The Market speaks its OWN ontology (mkt:) over schema:Product, labels
// things with the shared foaf:name, and resolves org URIs it does not
// own by querying the Directory's service over plain HTTP.

import { useEffect, useMemo, useState } from "react";
import {
  createClient,
  namesOf,
  recordsOf,
  resolverFromManifest,
  shortNamespace,
  type FactRecord,
} from "@demos/shared";
import { Badge } from "@demos/shared";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@demos/shared";
import { Skeleton } from "@demos/shared";
import { ShoppingBag } from "lucide-react";

const SELF = (import.meta.env.VITE_SEMWEB_URL as string | undefined) ?? "http://localhost:8486";
const DIRECTORY = (import.meta.env.VITE_DIRECTORY_URL as string | undefined) ?? "http://localhost:8484";
const BOARD = (import.meta.env.VITE_BOARD_URL as string | undefined) ?? "http://localhost:8487";

const TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PRODUCT = "http://schema.org/Product";
const NAME = "http://xmlns.com/foaf/0.1/name";
const SOURCED_FROM = "http://harborlight.example/market#sourcedFrom";
const ABOUT = "http://harborlight.example/board#about";

const self = createClient(SELF);
const directory = createClient(DIRECTORY);
const board = createClient(BOARD);

export default function App() {
  const [products, setProducts] = useState<string[]>([]);
  const [names, setNames] = useState<Map<string, string>>(new Map());
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [types, nameLines, manifest] = await Promise.all([
          self.fetchFragments({ predicate: TYPE, object: PRODUCT, limit: 100 }),
          self.fetchFragments({ predicate: NAME, limit: 200 }),
          self.fetchManifest(),
        ]);
        if (cancelled) return;
        setProducts(types.lines.map((l) => l["@id"] as string).filter(Boolean));
        setNames(namesOf(nameLines.lines, resolverFromManifest(manifest.predicates)));
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
            <ShoppingBag className="size-4 text-primary" aria-hidden="true" />
            Harborlight Market
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
            <h2 className="mb-3 text-sm font-medium text-muted-foreground">Coffee lots</h2>
            {products.length === 0 ? (
              <Skeleton className="h-40 w-full" />
            ) : (
              <ul>
                {products.map((p) => (
                  <li key={p}>
                    <button
                      type="button"
                      onClick={() => setSelected(p)}
                      className={`w-full border-b border-border px-2 py-2.5 text-left focus-visible:outline-2 focus-visible:outline-ring ${
                        selected === p ? "bg-secondary" : "hover:bg-secondary/50"
                      }`}
                    >
                      <span className="font-medium">{names.get(p) ?? shortNamespace(p)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <section>
            {selected ? (
              <ProductDetail uri={selected} />
            ) : (
              <p className="text-sm text-muted-foreground">
                Pick a lot. The sourcing org and the board chatter about it
                live in other apps' stores — resolved over plain HTTP.
              </p>
            )}
          </section>
        </div>
      </main>
    </div>
  );
}

interface ProductFacts {
  own: FactRecord[];
  sourcedFrom?: string;
  sourceName?: string;
  boardPosts: { id: string; title?: string }[];
}

function ProductDetail({ uri }: { uri: string }) {
  const [facts, setFacts] = useState<ProductFacts | null>(null);
  const loading = facts === null;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [manifest, page] = await Promise.all([
          self.fetchManifest(),
          self.fetchFragments({ subject: uri, limit: 50 }),
        ]);
        const resolve = resolverFromManifest(manifest.predicates);
        const own = recordsOf(page.lines, resolve);
        const sourcedFrom = own.find((r) => r.uri === SOURCED_FROM)?.value;
        // Cross-service read #1: the org URI belongs to the Directory.
        let sourceName: string | undefined;
        if (sourcedFrom) {
          const orgLines = await directory.fetchFragments({ subject: sourcedFrom, limit: 10 });
          const orgManifest = await directory.fetchManifest();
          sourceName = namesOf(orgLines.lines, resolverFromManifest(orgManifest.predicates)).get(sourcedFrom);
        }
        // Cross-service read #2: is this product on the Board?
        const posts = await board.fetchFragments({ predicate: ABOUT, object: uri, limit: 20 });
        const postTitles = await board.fetchFragments({ predicate: NAME, limit: 50 });
        const titles = namesOf(postTitles.lines, resolverFromManifest((await board.fetchManifest()).predicates));
        if (cancelled) return;
        setFacts({
          own,
          sourcedFrom,
          sourceName,
          boardPosts: posts.lines.map((l) => ({
            id: l["@id"] as string,
            title: titles.get(l["@id"] as string),
          })),
        });
      } catch {
        // cross-service reads are best-effort
        if (!cancelled) setFacts({ own: [], boardPosts: [] });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [uri]);

  const price = useMemo(
    () => facts?.own.find((r) => r.uri.endsWith("market#price"))?.value,
    [facts],
  );

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-xl">{facts ? factsLabel(facts, uri) : "…"}</CardTitle>
        <CardDescription className="break-all font-mono text-xs">{uri}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        {loading && <Skeleton className="h-24 w-full" />}
        {!loading && facts && (
          <>
            {price && (
              <p className="text-4xl font-semibold tracking-tight tabular-nums">
                €{price}
              </p>
            )}
            <div className="rounded-lg border border-border bg-background">
              {facts.own
                .filter((r) => !r.uri.endsWith("market#price"))
                .map((r, i) => (
                  <p key={i} className={`data-line px-4 py-2 ${i > 0 ? "border-t border-border" : ""}`}>
                    <span className="text-muted-foreground">{r.key}</span> {r.value}
                  </p>
                ))}
            </div>
            {facts.sourcedFrom && (
              <section className="flex flex-col gap-2">
                <Badge>Sourced from</Badge>
                <p className="text-sm">
                  <span className="font-medium">{facts.sourceName ?? facts.sourcedFrom}</span>
                  {facts.sourceName && (
                    <span className="ml-2 font-mono text-xs text-muted-foreground">
                      resolved from the Directory service ({facts.sourcedFrom})
                    </span>
                  )}
                </p>
              </section>
            )}
            {facts.boardPosts.length > 0 && (
              <section className="flex flex-col gap-2">
                <Badge variant="outline" className="w-fit border-signal/40 text-signal">
                  On the Board ({facts.boardPosts.length})
                </Badge>
                <ul className="flex flex-col gap-1 text-sm">
                  {facts.boardPosts.map((p) => (
                    <li key={p.id} className="border-b border-border py-1.5">
                      {p.title ?? p.id}
                    </li>
                  ))}
                </ul>
                <p className="text-sm text-muted-foreground">
                  Posts live in the Board app's store — found with one
                  fragment query against the Board's service.
                </p>
              </section>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

function factsLabel(facts: ProductFacts, uri: string): string {
  const name = facts.own.find((r) => r.uri === NAME)?.value;
  return name ?? shortNamespace(uri);
}