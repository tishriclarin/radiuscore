# RadiusCore

RadiusCore is the Rust RADIUS access, session accounting and IP resource layer. RadiusT owns subscriber service management and SystemT owns financial billing.

This source snapshot implements bounded legacy RADIUS framing and Option 82 decoding, configurable Rust rewrite rules and guarded lookup, IP prefix arithmetic and reply attributes, and the PostgreSQL core schema, dictionary, IP assignment lifecycle, and resolved accounting transaction. The CLI exposes offline diagnostics. Runtime transport authentication and UDP/TLS listeners, NAS policy provisioning, live Rust-to-PostgreSQL access/accounting, and the cluster/controller path are pending. See the engineering blueprint for the complete product design.

## Build and test

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
