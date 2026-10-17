# ADR-0013: Vocabulary is keyed per vertical, never merged

Status: Accepted
Date: 18 September 2026
Decision owner: Zubia Mughal

## Context

`vocabulary.toml` previously held one global `classifier_positive`,
`classifier_negative`, `outreach_lead`, and `outreach_avoid` list for
the whole config. Classifier terms for a fabricator (made to order,
quote from drawings) sat in the same list as terms for a restoration
firm (IICRC) and a homebuilder (guaranteed pricing).

The Vertical Differentiator Template is explicit about why that
shape is wrong: "If those lists are merged, the app writes an email
leading with electrification to a builder who scored highly because
his website mentions electrification. That is precisely the email
the client said not to send."

A merged list cannot tell a later outreach writer which vertical
produced the hit. A high score from one vertical's language is then
available to copy written for another.

## Decision

Vocabulary is keyed by `vertical_key`. Each `[[verticals]]` table in
`vocabulary.toml` holds that vertical's classifier and outreach
lists. Lists that the elicitation has not yet given are omitted,
not invented, and not borrowed from another vertical.

Lookup is by `vertical_key`. There is no global classifier list.

## Rejected alternatives

Rejected: keeping one merged classifier list and filtering at send
time. Rejected because the merged-list failure mode happens at
write time: the app already believes the hit belongs to the whole
product, not to one vertical.

Rejected: filling empty verticals from the old global list. Rejected
because that recreates the merge. Absence is the recorded state
until elicitation supplies terms.

## Consequences

`vocabulary.toml` is an array of per-vertical tables. Validation
checks classifier/outreach overlap inside a vertical, not across
the file. A vertical with no row has no vocabulary yet.
