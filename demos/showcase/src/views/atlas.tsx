// Atlas — demonstrates two documented claims:
//   1. "It describes itself"  — the manifest is generated from the live
//      store on every request; nothing here is cached or written by hand.
//   2. "Data comes in pages, streamed" — fragments over plain HTTP with
//      cursor pagination.

import { useCallback, useEffect, useState } from "react";
import {
  fetchFragments,
  fetchManifest,
  type Manifest,
  type ManifestClass,
  type ManifestPredicate,
} from "@/lib/semweb";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { ChevronRight, Fingerprint, RefreshCw } from "lucide-react";

export function Atlas({ url }: { url: string }) {
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setManifest(await fetchManifest());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const m = await fetchManifest();
        if (!cancelled) setManifest(m);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [url]);

  return (
    <div className="flex flex-col gap-10">
      <section className="flex flex-col gap-4">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <h2 className="text-2xl font-semibold tracking-tight text-balance">
            The description is generated, never written
          </h2>
          <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
            {loading ? <Spinner data-icon="inline-start" /> : <RefreshCw data-icon="inline-start" />}
            Refetch
          </Button>
        </div>
        <p className="max-w-prose text-sm text-muted-foreground">
          Everything below was regenerated from the live store when this page
          loaded — there is no build step and no cached copy. The schema
          fingerprint is a sha256 over the sorted class and predicate set:
          when it changes, subscribers to the schema topic know to re-plan.
        </p>
        {error && (
          <p className="font-mono text-sm text-destructive">
            {error} — is the service running on {url}?
          </p>
        )}
        {loading && !manifest && (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-8 w-72" />
            <Skeleton className="h-24 w-full" />
          </div>
        )}
        {manifest && <FingerprintRow manifest={manifest} />}
      </section>

      {manifest && (
        <>
          <ClassLedger manifest={manifest} />
          <PredicateLedger manifest={manifest} />
          <FragmentWorkbench manifest={manifest} />
        </>
      )}
    </div>
  );
}

function FingerprintRow({ manifest }: { manifest: Manifest }) {
  return (
    <div className="flex flex-col gap-1 border-y border-border py-3">
      <div className="flex items-center gap-2 text-sm font-medium">
        <Fingerprint className="size-4 text-primary" aria-hidden="true" />
        Schema fingerprint
      </div>
      <p className="data-line break-all">{manifest.schemaFingerprint}</p>
      {manifest.proof && (
        <p className="mt-2 text-sm text-muted-foreground">
          Signed under{" "}
          <span className="font-mono text-xs">{manifest.issuer}</span>{" "}
          <Badge variant="secondary">proof present</Badge>
        </p>
      )}
    </div>
  );
}

function ClassLedger({ manifest }: { manifest: Manifest }) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-4">
        <h3 className="text-lg font-semibold tracking-tight">Classes</h3>
        <p className="text-sm text-muted-foreground">
          live counts — {manifest.classes.reduce((n, c) => n + c.instances, 0)} instances
        </p>
      </div>
      <div>
        {manifest.classes.map((c) => (
          <ClassRow key={c.uri} c={c} />
        ))}
      </div>
    </section>
  );
}

function ClassRow({ c }: { c: ManifestClass }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="border-b border-border">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="grid w-full grid-cols-[auto_1fr_auto] items-center gap-4 py-3 text-left focus-visible:outline-2 focus-visible:outline-ring"
      >
        <ChevronRight
          className={`size-4 text-muted-foreground transition-transform ${open ? "rotate-90" : ""}`}
        />
        <span>
          <span className="font-medium">{c.compact ?? c.uri}</span>
          {c.description && (
            <span className="block text-sm text-muted-foreground">{c.description}</span>
          )}
        </span>
        <Badge variant="secondary">{c.instances} instances</Badge>
      </button>
      {open && (
        <div className="pb-4 pl-8">
          <p className="data-line mb-2 break-all text-muted-foreground">{c.uri}</p>
          {c.shapes && c.shapes.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {c.shapes.map((s) => (
                <Badge key={s.path} variant="outline" className="font-mono text-xs font-normal">
                  {s.path} min:{s.minCount} max:{s.maxCount ?? "∞"}
                </Badge>
              ))}
            </div>
          )}
          {c.exampleQuery && (
            <p className="data-line mt-3 rounded-md bg-muted p-3">{c.exampleQuery}</p>
          )}
        </div>
      )}
    </div>
  );
}

function PredicateLedger({ manifest }: { manifest: Manifest }) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-4">
        <h3 className="text-lg font-semibold tracking-tight">Predicates</h3>
        <p className="text-sm text-muted-foreground">
          live counts — {manifest.predicates.reduce((n, p) => n + p.triples, 0)} triples
        </p>
      </div>
      <div>
        {manifest.predicates.map((p) => (
          <div key={p.uri} className="grid grid-cols-[1fr_auto] items-center gap-4 border-b border-border py-3">
            <span>
              <span className="font-mono text-sm">{p.compact ?? p.uri}</span>
              {p.description && (
                <span className="block text-sm text-muted-foreground">{p.description}</span>
              )}
            </span>
            <Badge variant="secondary">{p.triples} triples</Badge>
          </div>
        ))}
      </div>
    </section>
  );
}

function FragmentWorkbench({ manifest }: { manifest: Manifest }) {
  const [predicate, setPredicate] = useState<string>(manifest.predicates[0]?.uri ?? "");
  const [subject, setSubject] = useState("");
  const [lines, setLines] = useState<Record<string, unknown>[]>([]);
  const [after, setAfter] = useState<string | undefined>();
  const [count, setCount] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const query = useCallback(
    async (cursor?: string) => {
      setLoading(true);
      setError(null);
      try {
        const page = await fetchFragments({
          subject: subject || undefined,
          predicate: predicate || undefined,
          after: cursor,
          limit: 5,
        });
        setLines((prev) => (cursor ? [...prev, ...page.lines] : page.lines));
        setAfter(page.after);
        setHasMore(page.hasMore);
        setCount(page.countEstimate);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    },
    [subject, predicate],
  );

  return (
    <section className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Fragment workbench</CardTitle>
          <CardDescription>
            A fragment is one GET request: fill any of the three positions,
            leave the rest as wildcards. One line of JSON per fact, cursor for
            the next page.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="flex flex-col gap-2">
              <Label htmlFor="fw-predicate">Predicate (uri or blank for wildcard)</Label>
              <Input
                id="fw-predicate"
                name="predicate"
                value={predicate}
                onChange={(e) => setPredicate(e.target.value)}
                className="font-mono text-xs"
                list="fw-predicates"
                autoComplete="off"
                spellCheck={false}
              />
              <datalist id="fw-predicates">
                {manifest.predicates.map((p: ManifestPredicate) => (
                  <option key={p.uri} value={p.uri}>
                    {p.compact}
                  </option>
                ))}
              </datalist>
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="fw-subject">Subject (uri or blank for wildcard)</Label>
              <Input
                id="fw-subject"
                name="subject"
                value={subject}
                onChange={(e) => setSubject(e.target.value)}
                placeholder="http://example.org/alice"
                className="font-mono text-xs"
                autoComplete="off"
                spellCheck={false}
              />
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Button onClick={() => void query()} disabled={loading}>
              {loading ? <Spinner data-icon="inline-start" /> : null}
              Fetch page
            </Button>
            <span className="text-sm text-muted-foreground">
              count estimate: <span className="font-mono">{count}</span>
            </span>
          </div>
        </CardContent>
      </Card>

      {error && <p className="font-mono text-sm text-destructive">{error}</p>}

      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <ScrollArea className="h-80">
          <div className="p-4">
            {lines.length === 0 && !loading && (
              <p className="text-sm text-muted-foreground">
                No lines yet — fetch a page above.
              </p>
            )}
            {lines.map((line, i) => (
              <div key={i} className="data-line whitespace-pre-wrap break-all py-1.5">
                {JSON.stringify(line)}
              </div>
            ))}
          </div>
          <ScrollBar orientation="horizontal" />
        </ScrollArea>
        {hasMore && (
          <div className="border-t border-border p-3">
            <Button variant="outline" size="sm" onClick={() => void query(after)} disabled={loading}>
              Next page
              <ChevronRight data-icon="inline-end" />
            </Button>
            <span className="ml-3 font-mono text-xs text-muted-foreground">
              cursor: {after?.slice(0, 24)}…
            </span>
          </div>
        )}
      </div>
      <p className="text-sm text-muted-foreground">
        That stream is the raw response body — the app adds nothing. Any HTTP
        client can read this database.
      </p>
    </section>
  );
}