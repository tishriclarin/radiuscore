# RadiusCore developer guide

RadiusCore is a focused Rust RADIUS service for ISP access networks. The repository contains working packet, rewrite, IPAM, PostgreSQL, and transport components. The production daemon, site-policy runtime, management CLI, accounting buffer and live log controls are design work until their integration tests pass. The [project rationale](project-rationale.md) explains why that scope was chosen.

## Build and test

From a repository checkout:

```sh
cargo fmt --check
cargo test --locked
cargo build --locked --release
```

Use Rust 1.80 or later, system OpenSSL development libraries and the OpenSSL CLI. The CLI generates temporary certificates for the TLS tests. An optional FreeRADIUS comparison uses the lab dependencies described in [Option 82 interoperability](option82-interop.md).

For PostgreSQL checks, provision a **fresh disposable database** on PostgreSQL 17 or later. Install the PostgreSQL client and Python 3 and provide connection settings through the normal PostgreSQL environment.

```sh
PGDATABASE=radiuscore_test ./scripts/test-postgres.sh
```

The script runs the migrations and checks allocation, leases, accounting projection, concurrency and retry behavior. These tests write into the selected database.

## Source map

| Path | Responsibility |
|---|---|
| [`src/main.rs`](../src/main.rs) | `radiuscore` diagnostic CLI, help and TLS probe. |
| [`src/radius.rs`](../src/radius.rs), [`src/option82.rs`](../src/option82.rs) | Packet length validation, selected attribute decoding and diagnostic conversions. |
| [`src/rewrite.rs`](../src/rewrite.rs) | Compiled bounded rewrite plans, per-worker scratch space and the required-rewrite gate. |
| [`src/integrity.rs`](../src/integrity.rs), [`src/radsec.rs`](../src/radsec.rs) | Legacy UDP Access-Request integrity and mutual TLS connection building blocks. |
| [`src/ipam.rs`](../src/ipam.rs) | CIDR parsing, free-prefix selection and RADIUS reply attribute encoding. |
| [`migrations/`](../migrations/) | Numbered PostgreSQL schema, attribute dictionary, address and lease constraints, and resolved accounting transaction. |
| [`tests/`](../tests/) | Rust, database, concurrency, TLS and interoperability fixtures. |
| [Engineering blueprint](engineering-blueprint.md) | Target daemon, access policy, topology, identity keyspaces, configuration generations, service ownership and accounting design. |

## Request boundary and concurrency

The target processing order is transport integrity and peer authorization, packet framing and attribute validation, vendor normalization, site and subscriber lookup, access policy, then reply encoding or accounting persistence. The diagnostic decoder can inspect untrusted datagrams because it cannot authorize or write live state. A production entry point must use the verified request path. Required rewrites must all succeed before a subscriber lookup. See [transport security](transport-security.md).

The runtime target is multi-threaded across CPU cores and multi-socket hosts. Compile peer dispatch, dictionaries and rewrite rules into immutable configuration generations. Reuse per-worker buffers and avoid a shared lock on the request hot path. Keep tables suitable for fast read access in memory where the database consistency model permits. Bound handoff queues and isolate I/O waiting from packet work. A request should retain a consistent configuration generation from receipt through reply.

Measure throughput, p95 and p99 latency, dropped requests, database pool wait, accounting batch latency and per-worker load. Record hardware, worker count, request mix and PostgreSQL settings. Separate packet-path, same-pool allocation and sustained accounting benchmarks. Publish performance claims only after repeatable measurements. Test CPU placement and contention before choosing affinity on multi-socket hosts.

## Sites, equipment and attributes

- Give each site a stable ID. Resolve NAS and OLT membership from trusted peer configuration and validated device mappings. A packet-provided site or equipment identifier cannot override the configured trust boundary.
- Normalize vendor circuit IDs, remote IDs and caller identifiers only under the selected device profile. Compile strict type, length, multiplicity, and allowlist checks for the attributes used in identity lookup or replies. Keep protocol integrity attributes outside ordinary rewrite rules.
- Build versioned lookup keys from typed, normalized components. Preserve the distinction between missing, empty and populated values. Scope a GPON lock to the relevant site, access node or OLT, and virtual port. Reject incomplete or ambiguous required tuples before identity lookup.
- Treat roaming as explicit eligibility for an allowed site or device group. Provision the mapping into the appropriate identity keyspace and test both allowed and denied moves, including when two OLTs expose the same port identifier.
- Add device support with sanitized captures or synthetic fixtures, the relevant attribute types and wire formats, expected normalized keys, and positive and negative reply tests. The initial support matrix should list the exact tested vendor, model, firmware and access method.

## Configuration and management integration

The planned Cisco-style shell and an embedded management CLI should share typed commands, authorization, and validation. Keep rendering independent so interactive and structured consumers receive the same semantics. A client can request configuration edits through a permissioned control interface. The daemon validates and publishes generations.

Validate the complete candidate, including peer scopes, dictionaries, access profiles, references and rewrite limits, before activation. Record the actor, previous revision and new semantic version. Use conflict detection for concurrent edits. Keep redacted configuration output, compare and diff operations, and rollback on the same authorization path. Conflicts and invalid candidates leave the running generation intact.

## PostgreSQL, IPAM and accounting

The core PostgreSQL schema is the authority for access identities, pool allocation and durable accounting. Constraints and transactions enforce non-overlapping active assignments within a routing space, retry-safe leases and accounting idempotency. Preserve those invariants when optimizing; same-pool contention or state that crosses workers requires concurrency tests.

The planned accounting writer uses a bounded buffer to combine rows into batches by size or time. Batching reduces write and transaction frequency. Authenticate, correlate and deduplicate records before persistence. Acknowledge only after the configured durability boundary, which is a successful PostgreSQL commit by default. An explicitly enabled durable spool can provide a separately configured boundary with different node-loss guarantees. Define high-water marks, backpressure, retries, replay and bounded retention. In-memory session-cache expiry must not discard uncommitted accounting records.

Test delayed and duplicate events, same-session ordering, full buffers, database outages, process restarts and replay. Add migrations for schema changes and verify upgrade behavior against test copies. Keep billing concerns in SystemT and subscriber service workflows in RadiusT.

## Logging and failure handling

Use a consistent event structure with severity, timestamp, component, trusted peer or site, correlation ID and a stable reason. Apply redaction before presentation or interactive filtering. Keep future live regex subscriptions bounded by expression length, compile cost and viewer queue size. A slow log viewer must not block authentication workers. Log management actions and health changes with high-signal messages. Never log credentials, key contents or raw subscriber identifiers.

The daemon design uses supervised workers, explicit shutdown, and observable degradation. Preserve bounded resource usage when a partner or database is slow. Refer to [transport security](transport-security.md) for peer trust, certificate lifecycle and the remaining deployment checks.

## Change checklist

1. Identify the affected packet, device, policy or database contract and preserve compatible behavior.
2. Use a sanitized fixture or synthetic input for risky protocol and parsing cases.
3. Run the Rust, SQL or device interop checks relevant to the change. Include concurrency and replay checks when the change crosses worker or transaction boundaries.
4. Update the [operator guide](operator-cli.md) and command help together for CLI changes. Mark planned behavior until production integration is tested.
5. Use the [contribution guide](../CONTRIBUTING.md) for review and the [security policy](../SECURITY.md) for privately reporting security issues.
