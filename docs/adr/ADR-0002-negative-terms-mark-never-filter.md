# ADR-0002: Negative terms mark, never filter

Status: Accepted
Date: 31 August 2026
Decision owner: Zubia Mughal

## Context

Two vocabulary lists exist per vertical: classifier terms and
outreach terms. The classifier_negative list is now populated from a
real conversation that went wrong. The pattern behind those terms is
agreement without authority: enthusiasm from someone who cannot
sign. The terms are our corporate, our national office, streamline
our processes, we can see how, and there is a gap unaccompanied by a
number.

The same question arose independently for business categories.
Extraction, aggregate and heavy civil businesses hold vacant land for
reasons unrelated to homebuilding and were initially proposed as an
exclusion. They were instead made categories.

## Decision

A classifier_negative term sets a flag on a row and produces a
bucket. It never removes a row from a result set. The same holds for
any business category: a category segments, it does not exclude.
Filtering remains a user action, never a system default.

## Rejected alternatives

Rejected: auto-excluding rows carrying a negative term. Rejected
because a default filter that removes prospects is worse than no
default. In the first vertical a default on one column hid two real
builders with live inventory, and the loss was invisible because a
removed row renders identically to a row that never existed.

## Consequences

Every negative term and every category needs a countable bucket in
the output. A row may carry several. The list length never shrinks
from classification alone.
