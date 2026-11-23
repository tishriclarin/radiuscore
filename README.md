# RadiusCore

![RadiusCore — RADIUS AAA, IPAM, and accounting](docs/assets/radiuscore-banner.jpg)

RadiusCore is a Rust RADIUS AAA project for ISPs that need to turn vendor-specific access requests into a consistent subscriber identity, IPv4 or IPv6 assignment, and session record. RadiusT manages subscriber services, and SystemT owns billing.

## What RadiusCore is trying to solve

An ISP can receive the same kind of access request through different NAS and access networks. One device may encode an Option 82 circuit ID as binary, another as text. In one rollout, the resulting subscriber identity must agree with the IP assignment and with the accounting record even when a request is retried or arrives late. RadiusCore is designed to keep these decisions consistent across sites.

- **Normalize before lookup.** Apply vendor-specific rules to identifiers such as Option 82 circuit ID, remote ID, and caller ID. If a required rewrite fails, stop before subscriber lookup.
- **Keep IP assignments consistent.** Use one PostgreSQL model for host addresses, delegated IPv6 prefixes, and routed blocks. Reject overlapping active assignments within a routing space and reuse the existing assignment on a matching retry.
- **Record reliable session history.** Preserve accounting events and update the session in one transaction. Duplicate events and late interim records follow explicit rules without reopening a closed session.

## How it differs

[FreeRADIUS](https://www.freeradius.org/radiusd/man/unlang.html) provides configurable packet policy through Unlang and [SQL address pools](https://www.networkradius.org/doc/current/raddb/mods-available/sqlippool.html). [Radiator](https://files.radiatorsoftware.com/radiator/ref/AuthByDYNADDRESS.html) also supports SQL-backed IPv4 address and IPv6 prefix allocation. RadiusCore's distinction is its focus on one ISP workflow: run vendor-specific identity normalization before lookup, allocate resources within a shared routing-space model, and keep accounting consistent with that session. The design also calls for versioned operator configuration with commit and rollback.

## Project status

The [transport security library](docs/transport-security.md) now provides mutual-TLS RadSec connection primitives, a TLS peer probe, and strict legacy UDP Access-Request verification.

**v0.1.0 is a development snapshot for lab use.** The Rust code implements legacy RADIUS packet framing, DHCP Option 82 decoding, rewrite rules with guarded lookup, IP prefix calculation, reply attribute encoding, and offline diagnostics. The PostgreSQL migrations cover IPv4 and IPv6 allocation and resolved session accounting. The live UDP/TLS AAA server, NAS provisioning, database runtime integration, and clustering are still in development.

| What you want to do | Where to start |
|---|---|
| Check TLS peer connectivity and security scope | [Transport security](docs/transport-security.md) |
| Understand the design and deployment plan | [Engineering blueprint](docs/engineering-blueprint.md) |
| Reproduce the FreeRADIUS and Option 82 checks | [Option 82 interoperability](docs/option82-interop.md) |
| Inspect address assignment and accounting | [IPAM migration](migrations/0003_ipam.sql) · [Accounting migration](migrations/0004_accounting.sql) |
| Report a bug or propose a change | [Contributing](CONTRIBUTING.md) · [Issues](https://github.com/tishriclarin/radiuscore/issues) |
| Report a security issue | [Security policy](SECURITY.md) |

## Build and test

Requires Rust 1.80+, system OpenSSL development libraries, and the OpenSSL CLI for transport tests.

```sh
cargo test --locked
cargo build --locked --release
cargo run -- option82 decode-dhcp 02074d595f41505f310106500000020005
cargo run -- ip parse 2001:db8:100::/56
PGDATABASE=radiuscore_test ./scripts/test-postgres.sh
```

The PostgreSQL script requires PostgreSQL 17 or later, a fresh disposable database, the PostgreSQL client and Python 3. It executes the migrations and validates in-pool checks, reservations, address and prefix overlap, routing-space isolation, retransmission idempotency, renewals and concurrent allocation. It also validates duplicate, out-of-order and late accounting events, unsigned counter limits and transaction rollback.

## IP resource model

| Table | Responsibility |
|---|---|
| `rc_ip_spaces` | Globally unique routing-domain code. A public routing space can be shared by multiple sites. Isolated VRFs can reuse addresses. |
| `rc_ip_pools` | Non-overlapping allocatable CIDR ranges per routing space, with an optional dynamic allocation length and a gate for new assignments. |
| `rc_ip_assignments` | Static and expiring host, link subnet, delegated-prefix and routed-prefix assignments, plus exclusions. Active assignments cannot overlap in the same routing space. |

For dynamic IPAM, lock the pool row before selecting a free prefix and keep that lock through `rc_ip_claim_dynamic`. The active claim key makes a retransmission for the same access account idempotent. If a claim expires, the resource remains reserved until the NAS or session state confirms it is no longer in use. `rc_ip_release_expired` compares the exact observed expiry so a stale cleanup worker cannot release a renewed assignment. Reserve unusable host ranges and infrastructure addresses explicitly as exclusions.

| Use | Standard RADIUS reply |
|---|---|
| Assign one IPv4 address | `Framed-IP-Address` |
| Assign one IPv6 address | `Framed-IPv6-Address` |
| Assign IPv4 host with link subnet | `Framed-IP-Address` and `Framed-IP-Netmask` |
| Assign an IPv6 link prefix | `Framed-IPv6-Prefix` |
| Delegate DHCPv6 prefix | `Delegated-IPv6-Prefix` |
| Route IPv4 or IPv6 block toward a subscriber | `Framed-Route` or `Framed-IPv6-Route` |

Check the NAS service's capabilities before sending these replies. RadiusCore supplies RADIUS attributes for the NAS to enforce. On RouterOS, validate route installation for the actual PPP or DHCP service. Upstream route advertisement remains a separate routing responsibility. Operator-configured NAS pool names can be represented in the reply dictionary. In that mode the NAS owns individual allocation and RadiusCore must not allocate from the same address space.

Use the public fixture details and field procedure in [Option 82 interop](docs/option82-interop.md). Test on the target RouterOS, ZTE and VSOL devices to confirm the exact values that arrive at RadiusCore and route behavior after reconnect.

[FreeRADIUS SQL IP pools](https://www.freeradius.org/documentation/freeradius-server/4.0.0/howto/modules/sqlippool/configure.html) · [RouterOS RADIUS](https://manual.mikrotik.com/docs/authentication-authorization-accounting/radius/) · [RADIUS attributes](https://www.iana.org/assignments/radius-types)
