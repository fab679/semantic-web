# Standards implemented

The infrastructure introduces no protocol of its own. Everything it
does is an existing standard; this page is the complete list, with
links to each specification.

## Reading and describing data

| Mechanism | Standard | Reference |
|---|---|---|
| Transport, methods, status codes, streaming | HTTP/1.1 | [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110), [RFC 9112](https://www.rfc-editor.org/rfc/rfc9112) |
| Data model (triples, named graphs) | RDF 1.1 | [W3C RDF 1.1](https://www.w3.org/TR/rdf11-concepts/) |
| Serialization (JSON lines carrying meaning) | JSON-LD 1.1 | [W3C JSON-LD 1.1](https://www.w3.org/TR/json-ld11/) |
| Paged access to a store's data | Triple Pattern Fragments (Linked Data Fragments) | [LDF / TPF](https://linkeddatafragments.org/) |
| Execution plane for complex queries | SPARQL 1.1 Protocol | [W3C SPARQL 1.1 Protocol](https://www.w3.org/TR/sparql11-protocol/) |
| Constraint shapes surfaced in the manifest | SHACL | [W3C SHACL](https://www.w3.org/TR/shacl/) |
| Simple (SKOS) and schema (RDFS) vocabularies for meanings | RDF Schema / SKOS | [RDFS](https://www.w3.org/TR/rdf-schema/), [SKOS](https://www.w3.org/TR/skos-reference/) |

## Push and live sessions

| Mechanism | Standard | Reference |
|---|---|---|
| Change notifications (hub, verification, leases, signed delivery) | WebSub | [W3C Recommendation](https://www.w3.org/TR/websub/) — the complete spec text is in [WebSub.md](WebSub.md) |
| Server-sent event stream for live sessions | SSE (EventSource) | [WHATWG HTML — server-sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html) |
| Delivery authentication | HMAC-SHA256 | [RFC 2104](https://www.rfc-editor.org/rfc/rfc2104) |

## Provenance (trust layer)

| Mechanism | Standard | Reference |
|---|---|---|
| Signed claims in VC 2.0 shape | Verifiable Credentials 2.0 | [W3C VCDM 2.0](https://www.w3.org/TR/vc-data-model-2.0/) |
| Document signatures | Data Integrity (Ed25519) | [W3C Data Integrity](https://www.w3.org/TR/vc-data-integrity/), [EdDSA cryptosuite](https://www.w3.org/TR/vc-di-eddsa/) |
| Web identity for the publisher | DID:web | [did:web method](https://w3c-ccg.github.io/did-method-web/) |
| Public key encoding | Multikey / multibase (base58btc) | [Multikey](https://www.w3.org/TR/vc-data-integrity/#multikey), [multibase](https://datatracker.ietf.org/doc/html/draft-sporny-multibase-01) |

## Operations

| Mechanism | Standard | Reference |
|---|---|---|
| Metrics | Prometheus text exposition | [Prometheus docs](https://prometheus.io/docs/instrumenting/exposition_formats/) |
| Signature-sealed secrets at rest | AES-256-GCM | [NIST SP 800-38D](https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf) |
| Token comparison (timing-safe) | Constant-time equality | [subtle crate](https://crates.io/crates/subtle) |

## Deliberate prototype simplifications (documented, not hidden)

- Canonicalization for signatures uses sorted-key JSON canonicalization
  rather than URDNA2015 — sound for the documents this service signs
  (no blank nodes), simpler to audit.
- Revocation uses a flat identifier list rather than the Bitstring
  Status List.
- The trust registry is operator-provisioned (environment or file)
  rather than a network-fetched, itself-signed registry.