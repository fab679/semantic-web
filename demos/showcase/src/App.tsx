// The shell: a claims ledger. The left rail lists the four documented
// claims of the infrastructure — they are a sequence — and each one is
// proven live by a workbench on the right.

import { useEffect, useState } from "react";
import { fetchHealth, SERVICE_URL } from "@/lib/semweb";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Atlas } from "@/views/atlas";
import { Pulse } from "@/views/pulse";
import { Verity } from "@/views/verity";

const CLAIMS = [
  { n: "1", claim: "Data comes in pages, streamed", demo: "atlas" },
  { n: "2", claim: "It describes itself", demo: "atlas" },
  { n: "3", claim: "It pushes", demo: "pulse" },
  { n: "4", claim: "Claims can be proven", demo: "verity" },
] as const;

export default function App() {
  const [healthy, setHealthy] = useState<boolean | null>(null);
  const url = SERVICE_URL;
  const [tab, setTab] = useState<string>(() => {
    const fromHash = window.location.hash.replace(/^#/, "");
    return CLAIMS.some((c) => c.demo === fromHash) || ["atlas", "pulse", "verity"].includes(fromHash)
      ? fromHash
      : "atlas";
  });

  useEffect(() => {
    const tick = () => void fetchHealth().then(setHealthy);
    tick();
    const t = setInterval(tick, 10_000);
    return () => clearInterval(t);
  }, []);

  function switchTab(next: string) {
    setTab(next);
    window.history.replaceState(null, "", `#${next}`);
  }

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-10 border-b border-border bg-background/90 backdrop-blur">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-3 px-6 py-3">
          <h1 className="text-lg font-semibold tracking-tight">
            Streaming Semantic Fragments
            <span className="ml-2 font-normal text-muted-foreground">— the claims, verified live</span>
          </h1>
          <div className="flex items-center gap-3">
            <a
              href={url}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-xs text-muted-foreground underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
            >
              {url}
            </a>
            <span
              className={`inline-block size-2 rounded-full ${
                healthy === null
                  ? "bg-muted-foreground"
                  : healthy
                    ? "bg-verified pulse-dot"
                    : "bg-destructive"
              }`}
              title={healthy ? "service healthy" : "service unreachable"}
              aria-label={healthy ? "service healthy" : "service unreachable"}
            />
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-6 py-8">
        <Tabs value={tab} onValueChange={switchTab}>
          <TabsList className="mb-8">
            <TabsTrigger value="atlas">Read</TabsTrigger>
            <TabsTrigger value="pulse">Watch</TabsTrigger>
            <TabsTrigger value="verity">Verify</TabsTrigger>
          </TabsList>

          <aside className="mb-8 grid gap-2 border-y border-border py-4 sm:grid-cols-2 lg:grid-cols-4">
            {CLAIMS.map((c) => (
              <p key={c.n} className="text-sm text-muted-foreground">
                <span className="mr-2 font-mono text-xs text-signal">{c.n}</span>
                {c.claim}
              </p>
            ))}
          </aside>

          <TabsContent value="atlas">
            <Atlas url={url} />
          </TabsContent>
          <TabsContent value="pulse">
            <Pulse url={url} />
          </TabsContent>
          <TabsContent value="verity">
            <Verity url={url} />
          </TabsContent>
        </Tabs>
      </main>

      <footer className="border-t border-border">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-2 px-6 py-4 text-sm text-muted-foreground">
          <p>
            Every number on this page came over plain HTTP, just now.
          </p>
          <p className="font-mono text-xs">GET / · GET /manifest · GET /fragments · GET /events · POST /mcp</p>
        </div>
      </footer>
    </div>
  );
}