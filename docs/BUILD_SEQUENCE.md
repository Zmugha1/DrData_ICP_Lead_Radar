# Build sequence

Phases run in order. Phase B is a gate rather than a step. Everything
in C, D and E is designed against what the nine lookups reveal.

## Phase A: config corrections

- [ ] Rename the band. Active builder becomes a label stating what
      the field holds: parcels with no improvement value. The
      interpretation moves to a separate description. A homeowners
      association currently sits inside the favourable band, which is
      the evidence.
- [ ] Ampersand canon. & becomes AND before the collapse, merging
      Payne and Dolan at two ranks. This is a spelling variant, not a
      suffix strip, and it does not reopen ADR-0001's normalisation
      decision.
- [ ] Load classifier_negative, with a validator rule enforcing
      ADR-0002.

## Phase B: the gate

Not a build step.

- [ ] Nine lookups with domain, verdict and reason.
- [ ] Two paragraphs in the owner's words: the ideal customer, and
      the walk-away.

## Phase C: enrichment schema

- [ ] The enrichment table per ADR-0003.
- [ ] Category as a config-driven bucket set covering property
      management, extraction and aggregate, heavy civil, homebuilder,
      specialty trade, manufacturer, and unknown.

## Phase D: enrichment fetch

- [ ] Domain store and manual entry.
- [ ] Page fetch with content normalisation.
- [ ] Page diff for the careers signal.
- [ ] Term matching, with a term_tested flag flipping true the first
      time a term actually appears on a real page.

## Phase E: extraction and narration

- [ ] Local extraction through Ollama, every output method Inferred.
- [ ] The prospect paragraph, counted facts and read facts assembled
      with a source and a date per sentence.

## Phase F: spine inversion

Per ADR-0004. Blocked on a purchase and a phone call.

- [ ] Wisconsin DFI business registrations as the entity spine.
      Parcels demote to an attachment.

## Phase G: plats

- [ ] An ADR first, recording that the layer carries no developer
      name and that the attribute is the most recent plat date on
      parcels currently owned.
- [ ] Then an adapter using centroid containment rather than polygon
      containment.

## Phase H: user interface

Last, because every surface is built against rules that have not
settled.

- [ ] Shell
- [ ] List page
- [ ] Coverage page
- [ ] Zone caveat rendering
- [ ] Export

## Open items

No owner in this repo.

- [ ] DFI Corporations call: full database price and format.
- [ ] DFI UCC call: whether the bulk media is a dated feed.
