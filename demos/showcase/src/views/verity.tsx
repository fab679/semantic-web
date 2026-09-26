// Verity — demonstrates the documented claim "Claims can be proven":
// the manifest carries a Data Integrity proof, the public key is served
// at /.well-known/did.json, and any signed document can be verified
// against four gates. Tamper with one byte and watch the math catch it.

import { useCallback, useEffect, useState } from "react";
import {
  fetchDid,
  fetchManifest,
  issueAttestation,
  verifyDocument,
  type DidDocument,
  type Manifest,
  type Verdict,
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
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { toast } from "@/components/ui/toast";
import {
  BadgeCheck,
  CircleX,
  Fingerprint,
  KeyRound,
  MinusCircle,
  ShieldCheck,
} from "lucide-react";

export function Verity({ url }: { url: string }) {
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [did, setDid] = useState<DidDocument | null>(null);
  const [verdict, setVerdict] = useState<Verdict | null>(null);
  const [tampered, setTampered] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const verify = useCallback(async (tamper: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const doc = await fetchManifest();
      setManifest(doc);
      const target = structuredClone(doc) as unknown as Record<string, unknown>;
      if (tamper && "schemaFingerprint" in target) {
        target.schemaFingerprint = "sha256:00000000000000000000000000000000evil";
      }
      setTampered(tamper);
      setVerdict(await verifyDocument(target));
    } catch (e) {
      setError(
        e instanceof Error
          ? `${e.message} — signed deployments need SEMWEB_SIGNING_KEY; is the service on ${url} configured for it?`
          : String(e),
      );
    } finally {
      setBusy(false);
    }
  }, [url]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const doc = await fetchManifest();
        if (cancelled) return;
        setManifest(doc);
        const target = structuredClone(doc) as unknown as Record<string, unknown>;
        setTampered(false);
        setVerdict(await verifyDocument(target));
      } catch (e) {
        if (cancelled) return;
        setError(
          e instanceof Error
            ? `${e.message} — signed deployments need SEMWEB_SIGNING_KEY; is the service on ${url} configured for it?`
            : String(e),
        );
      }
    })();
    fetchDid()
      .then((d) => {
        if (!cancelled) setDid(d);
      })
      .catch(() => {
        if (!cancelled) setDid(null);
      });
    return () => {
      cancelled = true;
    };
  }, [url]);

  return (
    <div className="flex flex-col gap-10">
      <section className="flex flex-col gap-4">
        <h2 className="text-2xl font-semibold tracking-tight">
          One byte, and the math notices
        </h2>
        <p className="max-w-prose text-sm text-muted-foreground">
          The manifest you are looking at was signed by the publisher under
          their web identity. Verification answers four separate questions —
          and the four answers tell you <em>why</em> something fails, not
          just that it did. Flip the switch to alter one value inside the
          document before verifying.
        </p>
        {error && <p className="font-mono text-sm text-destructive">{error}</p>}
      </section>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="size-4 text-primary" aria-hidden="true" />
              The live manifest
            </CardTitle>
            <CardDescription>
              {tampered
                ? "This copy has a forged fingerprint — compare the gate row."
                : "Fetched fresh from GET /manifest, exactly as a reader would."}
            </CardDescription>
          </div>
          <label className="flex cursor-pointer items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={tampered}
              onChange={(e) => void verify(e.target.checked)}
              className="accent-destructive"
            />
            tamper with one byte
          </label>
        </CardHeader>
        <CardContent className="flex flex-col gap-6">
          {manifest && (
            <>
              <GateRow verdict={verdict} busy={busy} />
              <div className="flex flex-col gap-2">
                <p className="text-sm font-medium">Schema fingerprint (the claim)</p>
                <p className="data-line break-all">{manifest.schemaFingerprint}</p>
                {manifest.proof && (
                  <p className="data-line break-all text-muted-foreground">
                    proofValue: {manifest.proof.proofValue}
                  </p>
                )}
              </div>
            </>
          )}
        </CardContent>
      </Card>

      {did && <DidPanel did={did} />}

      <AttestationBench />
    </div>
  );
}

function GateRow({ verdict, busy }: { verdict: Verdict | null; busy: boolean }) {
  if (busy && !verdict) return <p className="text-sm text-muted-foreground">Verifying…</p>;
  if (!verdict) return null;
  const gates = [
    { name: "shape", ok: verdict.gates.shape, note: "a well-formed proof is present" },
    { name: "signature", ok: verdict.gates.signature, note: "content matches the signature" },
    { name: "issuer trusted", ok: verdict.gates.issuerTrusted, note: "signer is trusted by this reader" },
  ] as const;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        {gates.map((g) => (
          <GateChip key={g.name} name={g.name} ok={g.ok} />
        ))}
        {verdict.gates.temporal !== null && (
          <GateChip name="fresh" ok={verdict.gates.temporal} />
        )}
        {verdict.gates.revocation !== null && (
          <GateChip name="not revoked" ok={verdict.gates.revocation} />
        )}
        <Badge variant={verdict.verified ? "secondary" : "destructive"}>
          {verdict.verified ? "verified — safe to use" : "refused — verify before use"}
        </Badge>
      </div>
      {verdict.details.length > 0 && (
        <ul className="flex flex-col gap-1">
          {verdict.details.map((d, i) => (
            <li key={i} className="data-line text-destructive">
              {d}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function GateChip({ name, ok }: { name: string; ok: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-sm font-medium ${
        ok
          ? "border-verified/30 bg-verified/10 text-verified"
          : "border-destructive/30 bg-destructive/10 text-destructive"
      }`}
    >
      {ok ? <BadgeCheck className="size-4" aria-hidden="true" /> : <CircleX className="size-4" aria-hidden="true" />}
      {name}
    </span>
  );
}

function DidPanel({ did }: { did: DidDocument }) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <KeyRound className="size-4 text-primary" aria-hidden="true" />
        <h3 className="text-lg font-semibold tracking-tight">
          The publisher's key, served over plain HTTP
        </h3>
      </div>
      <p className="max-w-prose text-sm text-muted-foreground">
        Identity is derived from the web location itself ({did.id}) — the
        same convention as a TLS certificate. Any conforming verifier can
        fetch this document and check signatures against it.
      </p>
      <div className="rounded-lg border border-border bg-card p-4">
        <p className="data-line break-all">{did.id}</p>
        {did.verificationMethod.map((vm) => (
          <p key={vm.id} className="data-line break-all pt-1 text-muted-foreground">
            {vm.publicKeyMultibase}
          </p>
        ))}
        {did.trust && (
          <p className="pt-2 text-sm text-muted-foreground">
            registry: {did.trust.trustedIssuers} trusted issuer
            {did.trust.trustedIssuers === 1 ? "" : "s"}, {did.trust.revokedCredentials} revoked
          </p>
        )}
      </div>
    </section>
  );
}

function AttestationBench() {
  const [claim, setClaim] = useState("The Huila lot #7 harvest is certified organic");
  const [issued, setIssued] = useState<Record<string, unknown> | null>(null);
  const [verdict, setVerdict] = useState<Verdict | null>(null);
  const [busy, setBusy] = useState(false);

  async function issue(validUntil?: string) {
    setBusy(true);
    try {
      const vc = await issueAttestation({
        claim: { says: claim },
        validUntil,
      });
      setIssued(vc);
      setVerdict(await verifyDocument(vc));
      toast.add({
        title: validUntil ? "Issued (already expired)" : "Issued",
        description: validUntil
          ? "Watch the freshness gate answer."
          : "A signed attestation — verify it like anything else.",
      });
    } catch (e) {
      toast.add({
        title: "Issuance failed",
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <Separator />
      <div className="flex items-center gap-2">
        <Fingerprint className="size-4 text-primary" aria-hidden="true" />
        <h3 className="text-lg font-semibold tracking-tight">
          Attestations: claims with expiry, revocable
        </h3>
      </div>
      <p className="max-w-prose text-sm text-muted-foreground">
        Any claim can be signed with a validity window. Issue a fresh one,
        then issue the same claim already expired — the freshness gate
        answers differently. In production, a stale attestation is
        superseded, not silently contradicted.
      </p>
      <Card>
        <CardContent className="pt-6">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="v-claim">Claim to attest</FieldLabel>
              <Input id="v-claim" name="claim" value={claim} onChange={(e) => setClaim(e.target.value)} />
              <FieldDescription>Signed under the publisher's DID with the current time.</FieldDescription>
            </Field>
            <div className="flex flex-wrap gap-3">
              <Button onClick={() => void issue()} disabled={busy}>
                Issue valid attestation
              </Button>
              <Button variant="outline" onClick={() => void issue("2020-01-01T00:00:00Z")} disabled={busy}>
                Issue expired attestation
              </Button>
            </div>
          </FieldGroup>
        </CardContent>
      </Card>
      {verdict && issued && (
        <div className="flex flex-col gap-2">
          <GateRow verdict={verdict} busy={busy} />
          <p className="data-line break-all text-muted-foreground">
            {JSON.stringify(issued).slice(0, 220)}…
          </p>
        </div>
      )}
      {!verdict && issued === null && (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <MinusCircle className="size-4" aria-hidden="true" /> nothing issued this session
        </p>
      )}
    </section>
  );
}