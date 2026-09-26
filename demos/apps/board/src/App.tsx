// Harborlight Signal Board (:5175 → service :8487)
//
// The Board speaks its OWN ontology (brd:) and owns none of the things
// it talks about: mentions point at people in the Directory's store,
// about points at products in the Market's store. Names resolve over
// plain HTTP — the shared foaf:name intersection doing real work. The
// feed is live: own-service SSE, no polling.

import { useCallback, useEffect, useState } from "react";
import {
  createClient,
  namesOf,
  recordsOf,
  resolverFromManifest,
  shortNamespace,
  type SseEvent,
} from "@demos/shared";
import { Badge } from "@demos/shared";
import { Button } from "@demos/shared";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@demos/shared";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@demos/shared";
import { Input } from "@demos/shared";
import { toast } from "@demos/shared";
import { Megaphone } from "lucide-react";

const SELF = (import.meta.env.VITE_SEMWEB_URL as string | undefined) ?? "http://localhost:8487";
const DIRECTORY = (import.meta.env.VITE_DIRECTORY_URL as string | undefined) ?? "http://localhost:8484";
const MARKET = (import.meta.env.VITE_MARKET_URL as string | undefined) ?? "http://localhost:8486";

const TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const NAME = "http://xmlns.com/foaf/0.1/name";
const NS = "http://harborlight.example/board#";

const self = createClient(SELF);
const directory = createClient(DIRECTORY);
const market = createClient(MARKET);

interface Post {
  id: string;
  title?: string;
  body?: string;
  postedAt?: string;
  mentions: string[];
  about?: string;
}

export default function App() {
  const [posts, setPosts] = useState<Post[]>([]);
  const [people, setPeople] = useState<Map<string, string>>(new Map());
  const [products, setProducts] = useState<Map<string, string>>(new Map());
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [postTypes, nameLines, manifest] = await Promise.all([
        self.fetchFragments({ predicate: TYPE, object: `${NS}Post`, limit: 100 }),
        self.fetchFragments({ predicate: NAME, limit: 200 }),
        self.fetchManifest(),
      ]);
      const resolve = resolverFromManifest(manifest.predicates);
      const postIds = postTypes.lines.map((l) => l["@id"] as string).filter(Boolean);
      const titles = namesOf(nameLines.lines, resolve);
      // details per post (small world; per-subject GETs are fine)
      const detailed = await Promise.all(
        postIds.map(async (id) => {
          const page = await self.fetchFragments({ subject: id, limit: 30 });
          const recs = recordsOf(page.lines, resolve);
          return {
            id,
            title: titles.get(id),
            body: recs.find((r) => r.uri === `${NS}body`)?.value,
            postedAt: recs.find((r) => r.uri === `${NS}postedAt`)?.value,
            mentions: recs.filter((r) => r.uri === `${NS}mentions`).map((r) => r.value),
            about: recs.find((r) => r.uri === `${NS}about`)?.value,
          } as Post;
        }),
      );
      detailed.sort((a, b) => (b.postedAt ?? "").localeCompare(a.postedAt ?? ""));
      setPosts(detailed);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // cross-service label resolution: one fragments call per neighbor
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [dirManifest, dirNames, mktManifest, mktNames] = await Promise.all([
          directory.fetchManifest(),
          directory.fetchFragments({ predicate: NAME, limit: 200 }),
          market.fetchManifest(),
          market.fetchFragments({ predicate: NAME, limit: 200 }),
        ]);
        if (cancelled) return;
        setPeople(namesOf(dirNames.lines, resolverFromManifest(dirManifest.predicates)));
        setProducts(namesOf(mktNames.lines, resolverFromManifest(mktManifest.predicates)));
      } catch {
        // neighbors offline: posts render with raw URIs
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // live: own-service SSE — new posts appear without any polling
  useEffect(() => {
    const close = self.openEvents(
      "/topics/data",
      (e: SseEvent) => {
        if (!e.error) void load();
      },
      setConnected,
    );
    return close;
  }, [load]);

  useEffect(() => {
    void (async () => {
      await load();
    })();
  }, [load]);

  return (
    <div className="min-h-screen">
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex max-w-5xl items-center justify-between gap-3 px-6 py-3">
          <h1 className="flex items-center gap-2 text-lg font-semibold tracking-tight">
            <Megaphone className="size-4 text-signal" aria-hidden="true" />
            Harborlight Signal Board
          </h1>
          <span className="flex items-center gap-2 text-sm text-muted-foreground">
            <span
              className={`inline-block size-2 rounded-full ${connected ? "bg-verified pulse-dot" : "bg-destructive"}`}
              aria-hidden="true"
            />
            {connected ? "live" : "connecting…"}
            <a href={SELF} className="font-mono text-xs underline-offset-4 hover:underline">
              {SELF}
            </a>
          </span>
        </div>
      </header>
      <main className="mx-auto grid max-w-5xl gap-8 px-6 py-8 lg:grid-cols-[1fr_360px]">
        <section className="flex flex-col gap-4">
          {error && <p className="font-mono text-sm text-destructive">{error}</p>}
          {posts.length === 0 && !error && (
            <p className="text-sm text-muted-foreground">No posts yet — write the first one.</p>
          )}
          {posts.map((p) => (
            <article key={p.id} className="border-b border-border pb-4">
              <div className="flex items-baseline justify-between gap-3">
                <h2 className="font-semibold">{p.title ?? shortNamespace(p.id)}</h2>
                <time className="font-mono text-xs text-muted-foreground">{p.postedAt}</time>
              </div>
              <p className="mt-1 text-pretty">{p.body}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {p.mentions.map((m) => (
                  <Badge key={m} variant="secondary" title={m}>
                    {people.get(m) ?? shortNamespace(m)}
                  </Badge>
                ))}
                {p.about && (
                  <Badge variant="outline" className="border-signal/40 text-signal" title={p.about}>
                    {products.get(p.about) ?? shortNamespace(p.about)}
                  </Badge>
                )}
              </div>
            </article>
          ))}
          <p className="text-sm text-muted-foreground">
            Names on the badges were resolved live from the Directory's and
            Market's stores — the Board only stores the posts, and the
            shared foaf:name convention names everything else.
          </p>
        </section>
        <Composer people={people} products={products} onPosted={() => void load()} />
      </main>
    </div>
  );
}

function Composer({
  people,
  products,
  onPosted,
}: {
  people: Map<string, string>;
  products: Map<string, string>;
  onPosted: () => void;
}) {
  const [body, setBody] = useState("");
  const [title, setTitle] = useState("");
  const [mentions, setMentions] = useState<string[]>([]);
  const [about, setAbout] = useState<string>("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    if (!body.trim()) return;
    setBusy(true);
    try {
      const id = `${NS}p${Date.now().toString(36)}`;
      const token = "demo-write-token";
      const now = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
      const triples = [
        { subject: id, predicate: TYPE, object: `${NS}Post` },
        { subject: id, predicate: `${NS}body`, object: body },
        { subject: id, predicate: `${NS}postedAt`, object: now },
        ...(title ? [{ subject: id, predicate: NAME, object: title }] : []),
        ...mentions.map((m) => ({ subject: id, predicate: `${NS}mentions`, object: m })),
        ...(about ? [{ subject: id, predicate: `${NS}about`, object: about }] : []),
      ];
      for (const t of triples) {
        const r = await self.insertTriple({ ...t, token });
        if (!r.ok) throw new Error(r.message);
      }
      toast.add({ title: "Posted", description: `${triples.length} facts written; the wire moved.` });
      setBody("");
      setTitle("");
      setMentions([]);
      setAbout("");
      onPosted();
    } catch (e) {
      toast.add({ title: "Post failed", description: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card className="self-start">
      <CardHeader>
        <CardTitle>Post to the board</CardTitle>
        <CardDescription>
          The Board writes only its own ontology — people and products are
          referenced by URI, owned by other apps.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="b-title">Title (shared foaf:name)</FieldLabel>
            <Input id="b-title" name="title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Cupping Friday…" />
          </Field>
          <Field>
            <FieldLabel htmlFor="b-body">Body</FieldLabel>
            <Input id="b-body" name="body" value={body} onChange={(e) => setBody(e.target.value)} placeholder="What is happening…" />
            <FieldDescription>Required. Written as brd:body.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel>Mentions (people from the Directory)</FieldLabel>
            <div className="flex flex-wrap gap-2">
              {[...people.entries()].map(([uri, name]) => (
                <button
                  key={uri}
                  type="button"
                  onClick={() =>
                    setMentions((prev) => (prev.includes(uri) ? prev.filter((m) => m !== uri) : [...prev, uri]))
                  }
                  className={`rounded-full border px-3 py-1 text-sm focus-visible:outline-2 focus-visible:outline-ring ${
                    mentions.includes(uri)
                      ? "border-primary bg-primary text-primary-foreground"
                      : "border-border hover:bg-secondary"
                  }`}
                >
                  {name}
                </button>
              ))}
              {people.size === 0 && <p className="text-sm text-muted-foreground">Directory unreachable — posting still works.</p>}
            </div>
          </Field>
          <Field>
            <FieldLabel htmlFor="b-about">About (products from the Market)</FieldLabel>
            <select
              id="b-about"
              name="about"
              value={about}
              onChange={(e) => setAbout(e.target.value)}
              className="h-9 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-2 focus-visible:outline-ring"
            >
              <option value="">— none —</option>
              {[...products.entries()].map(([uri, name]) => (
                <option key={uri} value={uri}>
                  {name}
                </option>
              ))}
            </select>
          </Field>
          <Button onClick={() => void submit()} disabled={busy || !body.trim()} className="w-full">
            {busy ? "Posting…" : "Post it"}
          </Button>
        </FieldGroup>
      </CardContent>
    </Card>
  );
}