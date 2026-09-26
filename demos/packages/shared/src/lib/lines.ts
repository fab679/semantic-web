// Helpers for rendering JSON-LD fragment lines as registry records.
//
// Fragment lines carry COMPACTED predicate keys (what the service's live
// prefix map produced, e.g. "name" for foaf:name). The manifest exposes
// the same compacting ({uri, compact} per predicate) — so the shared
// resolver maps each key back to its full URI, and ownership (which app
// wrote this fact) is decided on full URIs.

import { NAME, RDF, ownerOf, type Owner } from "./vocab";

export type Resolve = (compactKey: string) => string;

/** Build the compact-key -> full-URI resolver from the manifest's
 * predicate list (same compaction the fragment lines used). */
export function resolverFromManifest(predicates: { uri: string; compact?: string }[]): Resolve {
  const map = new Map(predicates.map((p) => [p.compact ?? p.uri, p.uri] as const));
  return (key) => map.get(key) ?? key;
}

/** Extract a displayable value from a JSON-LD value. */
export function valueOf(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    if ("@id" in o) return o["@id"] as string;
    if ("@value" in o) return String(o["@value"]);
  }
  return JSON.stringify(v);
}

export interface FactRecord {
  /** the compact key as the service wrote it (display form) */
  key: string;
  /** full predicate URI */
  uri: string;
  value: string;
  /** subject URI of the line this record came from */
  subject: string;
}

export function recordsOf(lines: Record<string, unknown>[], resolve: Resolve): FactRecord[] {
  const out: FactRecord[] = [];
  for (const line of lines) {
    const subject = (line["@id"] as string | undefined) ?? "";
    for (const [key, raw] of Object.entries(line)) {
      if (key === "@id" || key === "@context") continue;
      // rdf:type is serialized as the JSON-LD keyword @type
      const isType = key === "@type";
      const value = valueOf(raw);
      if (!value) continue;
      out.push({
        key: isType ? "rdf:type" : key,
        uri: isType ? RDF : resolve(key),
        value,
        subject,
      });
    }
  }
  return out;
}

export function groupByOwner(lines: Record<string, unknown>[], resolve: Resolve): Map<Owner, FactRecord[]> {
  const groups = new Map<Owner, FactRecord[]>();
  for (const record of recordsOf(lines, resolve)) {
    const owner = ownerOf(record.uri);
    if (!groups.has(owner)) groups.set(owner, []);
    groups.get(owner)!.push(record);
  }
  return groups;
}

/** subject URI -> foaf:name (the shared intersection predicate). */
export function namesOf(lines: Record<string, unknown>[], resolve: Resolve): Map<string, string> {
  const names = new Map<string, string>();
  for (const line of lines) {
    const subject = (line["@id"] as string | undefined) ?? "";
    for (const [key, raw] of Object.entries(line)) {
      if (key === "@id" || key === "@context") continue;
      if (resolve(key) === NAME && subject) names.set(subject, valueOf(raw));
    }
  }
  return names;
}

/** rdf:type object URIs found in lines (what a subject is). */
export function typesOf(lines: Record<string, unknown>[], resolve: Resolve): string[] {
  const types: string[] = [];
  for (const record of recordsOf(lines, resolve)) {
    if (record.uri === RDF) types.push(record.value);
  }
  return types;
}

export { shortNamespace as compactUri, ownerOf, type Owner } from "./vocab";

export function ownerBadgeLabel(owner: Owner): string {
  switch (owner) {
    case "shared":
      return "shared vocabulary";
    case "directory":
      return "written by Directory";
    case "market":
      return "written by Market";
    case "board":
      return "written by Board";
    default:
      return "another vocabulary";
  }
}