// Pulse — demonstrates the documented claim "It pushes": change
// notifications arrive the moment data changes (SSE mirror of the WebSub
// bus). Insert a fact below and watch the wire — no polling anywhere.

import { useEffect, useRef, useState } from "react";
import {
  insertTriple,
  openEvents,
  type SseEvent,
} from "@/lib/semweb";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { toast } from "@/components/ui/toast";
import { RadioTower, Zap } from "lucide-react";

export function Pulse({ url }: { url: string }) {
  const [events, setEvents] = useState<SseEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const counter = useRef(0);

  useEffect(() => {
    const close = openEvents(
      "/topics/data",
      (e) => {
        counter.current += 1;
        setEvents((prev) => [{ ...e, eventId: e.eventId ?? `#${counter.current}` }, ...prev].slice(0, 40));
      },
      setConnected,
    );
    return close;
  }, [url]);

  return (
    <div className="grid gap-10 lg:grid-cols-[1fr_380px]">
      <section className="flex flex-col gap-4">
        <div className="flex items-baseline justify-between gap-4">
          <h2 className="text-2xl font-semibold tracking-tight">The change wire</h2>
          <span className="flex items-center gap-2 text-sm text-muted-foreground">
            <span
              className={`inline-block size-2 rounded-full ${connected ? "bg-verified pulse-dot" : "bg-destructive"}`}
              aria-hidden
            />
            {connected ? "listening to /topics/data" : "connecting…"}
          </span>
        </div>
        <p className="max-w-prose text-sm text-muted-foreground">
          This is the server-sent-events mirror of the WebSub hub. Every
          mutation anywhere in the graph lands here within milliseconds.
          Use the form to publish one and watch it arrive — there is no
          polling loop in this page.
        </p>
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          {events.length === 0 ? (
            <p className="p-6 text-sm text-muted-foreground">
              Quiet so far. Publish a fact from the form and the wire moves.
            </p>
          ) : (
            <ol>
              {events.map((e, i) => (
                <li
                  key={`${e.eventId}-${e.receivedAt.getTime()}`}
                  className={`data-line flex flex-wrap items-center gap-3 px-4 py-2.5 ${i === 0 ? "line-arrive" : ""}`}
                >
                  <span className="text-muted-foreground">
                    {e.receivedAt.toLocaleTimeString()}
                  </span>
                  {i === 0 && (
                    <Zap className="size-3.5 text-signal" aria-hidden="true" />
                  )}
                  <span className="font-medium">{e.eventId}</span>
                  <span className="text-muted-foreground">{e.topic ?? e.error ?? e.raw}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
        {events.length > 0 && (
          <p className="text-sm text-muted-foreground">
            {events.length} event{events.length === 1 ? "" : "s"} received this session.
          </p>
        )}
      </section>

      <PublishForm />
    </div>
  );
}

function PublishForm() {
  const [subject, setSubject] = useState("http://example.org/erin");
  const [predicate, setPredicate] = useState("http://xmlns.com/foaf/0.1/knows");
  const [object, setObject] = useState("http://example.org/alice");
  const [token, setToken] = useState("demo-write-token");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    try {
      const result = await insertTriple({ subject, predicate, object, token });
      if (result.ok) {
        toast.add({
          title: result.duplicate ? "No change" : result.schemaChanged ? "Schema changed" : "Published",
          description: result.message,
        });
      } else {
        toast.add({ title: "Write rejected", description: result.message });
      }
    } catch (e) {
      toast.add({
        title: "Write failed",
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card className="self-start">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <RadioTower className="size-4 text-signal" aria-hidden="true" />
          Publish one fact
        </CardTitle>
        <CardDescription>
          A POST with three strings. The hub fans the change out to every
          subscriber — including the wire on the left.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="p-subject">Subject</FieldLabel>
            <Input id="p-subject" name="subject" className="font-mono text-xs" value={subject} onChange={(e) => setSubject(e.target.value)} autoComplete="off" spellCheck={false} />
          </Field>
          <Field>
            <FieldLabel htmlFor="p-predicate">Predicate</FieldLabel>
            <Input id="p-predicate" name="predicate" className="font-mono text-xs" value={predicate} onChange={(e) => setPredicate(e.target.value)} autoComplete="off" spellCheck={false} />
            <FieldDescription>New predicate? A schema event fires too.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="p-object">Object</FieldLabel>
            <Input id="p-object" name="object" className="font-mono text-xs" value={object} onChange={(e) => setObject(e.target.value)} autoComplete="off" spellCheck={false} />
          </Field>
          <Field>
            <FieldLabel htmlFor="p-token">Write token</FieldLabel>
            <Input id="p-token" name="token" type="password" className="font-mono text-xs" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
          </Field>
          <Button onClick={() => void submit()} disabled={busy} className="w-full">
            {busy ? "Publishing…" : "Insert and push"}
          </Button>
          <p className="text-sm text-muted-foreground">
            Insert the same triple twice: the second write changes nothing
            and pushes nothing.
          </p>
        </FieldGroup>
      </CardContent>
    </Card>
  );
}