# ADR-0016: Brew City hospitality vertical, source ladder, private publish route

Status: Proposed
Date: 28 October 2026
Decision owner: Zubia Mughal

Number note: ADR-0015 is left free on purpose. A stale-row ADR draft
exists and is not approved. This ADR does not depend on it.

## Context

Brew City PC (an MSP) wants a weekly list of new hospitality
businesses in Milwaukee, with its existing customers protected. The
list is published to a small Netlify app of its own.

What the work on 28 October 2026 showed:

- The liquor licence snapshot diff flagged 35 of 1,269 rows as new.
  Twenty of the 35 were checked against news, business listings and
  the Licenses Committee record. Eight look like new businesses. Ten
  are established places that added or changed a licence, or a
  seasonal event licence. Two are unverified. The other 15 were set
  aside as likely re-keys, added licences or retail, and were not
  individually checked beyond the snapshot comparison.
- The licence file has no licence number. The location key is a
  taxkey, and 45 taxkeys map to more than one address (118 rows).
  Renewals reset effective dates: 122 of 1,214 renewals carried a
  later date.
- The Licenses Committee record is earlier than the licence file. For
  seven businesses, the hearing came 3 days to about 10 weeks before
  the licence row's effective date. Six of the eight likely-new
  businesses appear in it. Two do not appear in the lists read.
- A tavern application is not a new business. A wine shop and a
  grocery store also appear in the committee record.
- The committee record is on `milwaukee.legistar.com`, one meeting
  page per meeting. Each application is a free-text minutes note
  that also names individuals. Only business fields are needed.
- The Pulse app publishes the ranked CSV by reading it from a GitHub
  raw URL on `main` with no token. That route has no access control.
- The README says adding a vertical is a new config directory, never
  a code change (ADR-0001).

## Decision

1. The Brew City vertical is a config directory,
   `config/brewcity-hospitality/`. Code that every vertical can use
   (a committee-record adapter, a stage model) is written on a
   feature branch and merged to `main`. No long-lived client branch.

2. Sources are ranked by how early they show a business. The
   evidence tier is stored on each lead.
   1. Licenses Committee record (earliest, labels new, transfer and
      renewal).
   2. Public news coverage of licence hearings.
   3. The liquor licence snapshot diff (confirms that the licence
      issued, tracks closures).
   4. A business listing lookup, by hand, per lead.

3. A lead is one business, keyed on a normalized address with the
   taxkey as fallback. Fields: trade name, entity name only when it
   ends in LLC, INC, CORP, CORPORATION, CO, COMPANY, LTD, LP, LLP or
   LLLP, address, licence types, stage, stage dates, evidence tier,
   source link, business status (likely new, established,
   unverified), contact fields, customer flag.
   Stage and status are observations. They are inserted, never
   updated, and a view resolves the current value. Unknown stays
   blank. "Likely new" is a label with a source. It is not a
   prediction.

4. Customer protection happens on the owner's machine. The customer
   list is matched on normalized names and addresses. Matched leads
   are marked, and removed from the exported file for the client. The
   customer list is never committed, never sent to a cloud service,
   and never pasted into a chat. The export reports the match count.
   A match step that matches nothing is treated as a failure until
   proven otherwise.

5. Publishing goes to a private store that only the client's app can
   read, one tenant per client. It is never this repository and never
   a public raw URL. The exact store (a Supabase table written by a
   script, or a private repository read through a Netlify function
   that holds a token) is decided in a follow-up ADR before any
   publish code is written.

6. Contact fields (phone, website, email) are entered by hand with a
   source and a date. No purchased contact data, no skip trace, no
   people search, no reverse lookup. Person names from licence,
   occupancy and committee records are not stored or exported.

## Rejected alternatives

- A long-lived branch in this repository for the client. Rejected
  because it forks the code. A fix on `main` would not reach it, and
  it contradicts ADR-0001.
- Publishing through the public GitHub raw URL, as the Pulse ranked
  CSV does. Rejected because the file would be readable by anyone
  with the link, and the client's list is not for the public.
- Cloning the Pulse app for the client. Rejected because Pulse holds
  the owner's own invoices and P&L.
- The licence file as the main source. Rejected because it is late
  and noisy. About one in four flagged rows was a likely new business
  after checking.
- Treating any tavern application as a new business. Rejected
  because established stores and restaurants also apply.
- Buying contact data. Rejected as a standing rule.

## Consequences

- Retrieval of the committee record stays human-assisted until the
  host's terms are checked. Other Milwaukee hosts in this system
  forbid automated agents (see the README on LIRA). The Legistar host
  has not been checked. Until it is, a person saves the pages into a
  folder and the loader reads the folder, the same pattern as LIRA.
  Before any automated pull: read robots.txt and the terms, set a
  rate limit, and record the result in a follow-up ADR.
- A read-only profile of every Licenses Committee meeting from January
  to date comes before any adapter. It reports item counts, fill
  rates per field and parse failures. This follows the rule to profile
  a source with real requests before writing an adapter.
- The result field needs strict parsing. The action tag AMENDED is
  not approval. Approval was verified for three applications only.
- Completeness of the committee record is unmeasured. Two
  likely-new businesses were absent from the lists read.
- Outreach compliance is unchecked: the Wisconsin No Call list for
  business lines, CAN-SPAM and TCPA. A Wisconsin business attorney
  reviews before the client uses the list for outreach.
- The weekly refresh runs on the owner's machine, so the machine must
  be on. A scheduler decision is deferred.
- The build price quoted to the client may understate the source
  work. Recheck after the profile.
- Every guard added for this vertical is shown failing once before it
  is trusted: break the thing it guards, show the failure, revert,
  show the pass.
