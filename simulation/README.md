# RadiusCore network simulation

This lab exercises RadiusCore's Rust packet and admission components and the real PostgreSQL functions used for session admission, IP allocation, and accounting. It runs entirely in process plus an optional disposable PostgreSQL database. The live AAA daemon is still pending. The lab opens no network listener and emits no high-volume traffic.

## Run the simulation

```sh
./simulation/run.sh
PGDATABASE=radiuscore_test ./simulation/run.sh --database --seed 27 > result.json
```

The plain invocation reports local results and marks PostgreSQL skipped. For the full run, configure the standard PostgreSQL connection variables and point `PGDATABASE` at a disposable database with migrations 0001–0005 already applied. To initialize and test a fresh disposable database in one command, run `PGDATABASE=radiuscore_test ./scripts/test-postgres.sh`. This command also runs the existing database tests. The standalone simulation rolls fixture changes back. It accepts an optional reproducible unsigned seed.

Standard output from the simulation is JSON with overall `status`, a summary, and each test's `expected`, `actual`, and `status`. A failed comparison or database run exits nonzero. A local run with PostgreSQL skipped reports `partial`. Build errors occur before the reporting executable starts.

## Coverage

| Scenario | Expected behavior |
|---|---|
| Valid and invalid requests | Verify the signed Access-Request fixture. Reject truncated framing, malformed attribute lengths, tampered values, and bad IP prefixes. |
| Ordered sessions | Seed-shuffle the accounts, then reserve an admission, allocate an IP, record Start and Interim, and close on Stop for each account. |
| Out-of-order sessions | Handle Stop before Start, delayed Interim, and duplicate Stop without reopening the closed session or recording the duplicate event twice. |
| Unknown or unauthenticated NAS | Reject without creating a session bucket or entering subscriber, IP, or accounting work. |
| Per-NAS and per-session quotas | Exercise admission decisions with a fixed logical clock. Reject when the session or NAS exhausts its budget, keep rate state bounded, and refill or reclaim idle buckets. |
| Abrupt disconnect | Detect open rows when no Stop arrived. Follow a fixture that supplies verified absence, then reconcile the accounting row and release the admission slot. |
| DHCP T1 at one simultaneous user | Keep the same slot and IP on the same verified client, extend them after confirmed renewal, reject a different client, and ignore a stale teardown carrying the old expiry. |
| Deactivated account | Deny both retries and new admissions after an access account is disabled. Active session termination follows authorized device reconciliation. |
| Expired lease with no Stop | Keep the old slot held until authoritative expiry is reconciled, then admit a new context. |

## Runtime policy and security

- Before cryptographic work, reject unknown peers where the transport permits it. On routed sites use network exposure controls and an upstream rate limit. Broad or permanent automatic IP blocklists can misidentify spoofed or shared addresses. Prefer configured allowlists, bounded transient throttling for unverified traffic, handshake and connection caps, and operator-reviewed blocks.
- Use the **verified** NAS identity for each NAS budget. Apply a session budget using a stable verified and scoped key because an Access-Request may not carry an accounting session ID. A peer-supplied raw ID cannot select arbitrary state. If the key is unavailable, the NAS budget remains active. Place limits before database, IPAM, and accounting queue work. Keep session-ending accounting capacity available under access load.
- The example limiter is local to the caller. The production daemon needs consistent per-NAS ownership or a shared quota across workers and nodes, plus metrics and a separate bounded aggregate ingress budget. Application limits protect CPU work after admission, database capacity, and disk writes. Upstream controls are needed when traffic saturates a link.
- `rc_access_accounts.simultaneous_users` is a local account policy. `NULL` means unlimited. Apply the limit to distinct active service contexts, including provisional admission before Accounting-Start. Serialize admissions per account in PostgreSQL at READ COMMITTED so the locked count sees any admission that just committed. A retransmission reuses its existing admission. First-time reservations whose service never starts need a runtime reconciliation policy.
- For DHCP, bind the admission to the trusted NAS or access domain, the normalized Option 82 line identity, and the stable client identity. A 10-minute lease normally renews around minute five. A confirmed renewal for the same verified binding retains its slot, renews the IP and extends the lease deadline. Update the lease and admission together in a transaction when both are managed in RadiusCore. End only after authoritative expiry, verified release, or confirmed disappearance. Keep the admission generation and previously observed expiry on cleanup work so old events cannot free a newer session.
- Treat stale rows as candidates. For PPP or other sessions without authoritative expiry, confirm with the trusted NAS before reconciling. Persist matching identifiers, apply the accounting and admission closures together, and release IP only once its own authoritative state permits reuse. The supplied SQL primitives and fixtures support those checks. Device reconciliation, durable buffering, and live request routing still require daemon integration.

[DHCP renewal and expiration](https://www.rfc-editor.org/rfc/rfc2131.html#section-4.4.5) · [Transport security](../docs/transport-security.md)
