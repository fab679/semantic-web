// The three app ontologies and their shared intersection. Each app OWNS
// its namespace; the intersection is what lets them interoperate without
// knowing about each other.
//
//   Directory  speaks  foaf: + schema:worksFor/employee/foundingDate
//   Market     speaks  mkt: on schema:Product instances
//   Board      speaks  brd: posts about people and products
//
// Shared (the intersection):
//   foaf:name   — every app labels the things it creates with it
//   rdf:type    — every app classifies with it
//   rdfs:comment — every app documents its terms with it

export type Owner = "shared" | "directory" | "market" | "board" | "other";

export const RDF = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
export const NAME = "http://xmlns.com/foaf/0.1/name";

export const NS = {
  directory: {
    person: "http://xmlns.com/foaf/0.1/Person",
    organization: "http://xmlns.com/foaf/0.1/Organization",
    knows: "http://xmlns.com/foaf/0.1/knows",
    worksFor: "http://schema.org/worksFor",
    employee: "http://schema.org/employee",
    foundingDate: "http://schema.org/foundingDate",
  },
  market: {
    ns: "http://harborlight.example/market#",
    product: "http://schema.org/Product",
    price: "http://harborlight.example/market#price",
    harvestDate: "http://harborlight.example/market#harvestDate",
    certifiedBy: "http://harborlight.example/market#certifiedBy",
    sourcedFrom: "http://harborlight.example/market#sourcedFrom",
  },
  board: {
    ns: "http://harborlight.example/board#",
    post: "http://harborlight.example/board#Post",
    postOf: "http://harborlight.example/board#post",
    body: "http://harborlight.example/board#body",
    postedAt: "http://harborlight.example/board#postedAt",
    mentions: "http://harborlight.example/board#mentions",
    about: "http://harborlight.example/board#about",
  },
} as const;

/** Which app a predicate URI belongs to — the "who wrote this" badge. */
export function ownerOf(uri: string): Owner {
  if (uri === NAME || uri === RDF || uri.startsWith("http://www.w3.org/2000/01/rdf-schema")) {
    return "shared";
  }
  if (uri.startsWith("http://xmlns.com/foaf") || uri.startsWith("http://schema.org")) {
    return "directory";
  }
  if (uri.startsWith(NS.market.ns)) return "market";
  if (uri.startsWith(NS.board.ns)) return "board";
  return "other";
}

export function shortNamespace(uri: string): string {
  if (uri.startsWith(NS.market.ns)) return `mkt:${uri.slice(NS.market.ns.length)}`;
  if (uri.startsWith(NS.board.ns)) return `brd:${uri.slice(NS.board.ns.length)}`;
  if (uri.startsWith("http://xmlns.com/foaf/0.1/")) return `foaf:${uri.slice("http://xmlns.com/foaf/0.1/".length)}`;
  if (uri.startsWith("http://schema.org/")) return `schema:${uri.slice("http://schema.org/".length)}`;
  if (uri === RDF) return "rdf:type";
  if (uri.startsWith("http://www.w3.org/2000/01/rdf-schema#")) return `rdfs:${uri.slice("http://www.w3.org/2000/01/rdf-schema#".length)}`;
  return uri;
}