# What it enables

Infrastructure is judged by what becomes *cheap*. Here is what gets
cheap when live data can be served, self-described, pushed, and
proven.

---

## Publishing data becomes as cheap as publishing a page

Today, exposing a dataset to the web means an API: a product with an
engineering team, documentation, auth, versioning, and a support
contract. That cost is why most of the world's data is invisible.

When serving is configuration instead of construction — point the
service at a store, done — the publishers who appear are not the ones
with API teams. They are the long tail: municipalities publishing
budgets, researchers publishing datasets, co-ops publishing prices,
archives publishing catalogs, hospitals publishing wait times.

**The precedent:** cheap document publishing reshaped media, commerce,
and politics within a decade. Nobody knows which *data* publishers
emerge when the cost drops to zero — that's the point.

## Data stops going stale

The web of data today runs on exports: CSV dumps, nightly scrapes,
point-in-time copies that drift from their source. Every integration
is a sync job; every sync job is a future outage.

With push distribution (WebSub) and live self-description, the served
surface *is* the source. Subscribers receive changes the moment they
happen. "Let me send you the latest file" becomes an antique phrase.

**The precedent:** RSS took news from "I check sites on a timer" to
"new articles arrive." The same shift for prices, schedules, gauges,
and statistics removes an entire class of staleness bugs — and an
entire class of scraping load.

## Small publishers keep direct relationships

Data distribution currently runs through platform APIs that are also
chokepoints: terms you accept, keys you hold, access you can lose.
A standard hub-and-subscriber fabric has no such middle: any publisher
is discoverable by Link header, any subscriber can follow, nobody is
granted or revoked from the middle.

**The precedent:** this is the property that made email and the
document web hard to fully capture. Distribution without a toll booth
is a structural thing, not a feature request.

## Information can carry its own evidence

Almost everything consumed digitally today — statistics, prices,
ingredients, statements — arrives with no verifiable attachment of
*who vouched for it and when*. Fact-checking happens after the fact,
on copies, for a few contested claims.

When provenance is part of the medium — signed at publication,
checkable in one request — verification moves **before use**, and
stops being a luxury good. A reader (a journalist, an auditor, a
regulator, an application, a citizen) can refuse anything that arrives
naked.

**The precedent:** HTTPS made transport security a default nobody
thinks about. The same trajectory for claim provenance is how
misinformation defenses stop being reactive.

## The data web compounds instead of fragmenting

Every integration built on bespoke APIs is a bilateral agreement; the
network effect dies at the second participant. Standards compose by
construction: two publishers speaking the same conventions can be
combined by *any* reader without a meeting. Every new node raises the
value of every other node — the original property of the web itself,
recreated for data.

---

## The honest part

None of this happens by being published. Standards win when adopting
them is cheaper than the status quo *for the publisher* — and the
historical blocker was always incentive, not technology. The trust
layer is the incentive mechanism: verifiable data is *more valuable*
data, and a growing body of regulation (transparency, provenance,
reporting duties) creates demand for exactly that. The realistic
sequence is narrow first — places where provenance is already legally
required — then the compounding everyone else gets for free.

## What this is not

It is not a platform, not a marketplace, and not a network you join.
It is a set of conventions and a small reference implementation for
serving and receiving data the way the web serves and receives
documents. Platforms can be captured; conventions with reference
implementations are closer to weather.