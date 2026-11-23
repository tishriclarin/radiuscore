# RadiusCore engineering blueprint

**Status:** Proposed PostgreSQL baseline and Rust implementation inventory. The core baseline SQL defines 16 tables. The implementation package adds IP resource management, bringing the core to 19 tables across four migrations, and extends the RADIUS dictionary. The repository blueprint covers both implemented modules and planned deployment and runtime components.

## Design decisions

- Single production executable named `radiuscore`. Run it standalone now. A local control interface lets Axion manage it later.
- PostgreSQL is authoritative for network access accounts, registered sites and devices, credentials, service profiles, the dictionary, cluster configuration revisions, and durable accounting. Rust workers use immutable in-memory generations for hot reads.
- Match, parse, and rewrite are configured per NAS and packet stage. On a valid packet, choose the plan with one constant-time access to the resolved peer or trusted downstream slot. A required rewrite failure ends the request's lookup path with zero identity, cache, or SQL lookup calls.
- Candidate edits, commit, autocommit, and rollback operate on a versioned configuration manifest. The same manifest carries vendor defaults, peer bindings, rules, access policies, and cluster-wide settings. Local filesystem configuration supplies node-specific startup and recovery details.
- A compact PostgreSQL model uses relational columns for fields that need constraints or indexes. JSONB is used for the versioned manifest and provisioned identity selectors whose fields vary by keyspace. SQL types reflect actual ranges. A limit of 200 values is stored in `smallint` with a `CHECK`; PostgreSQL's built-in integers start at two bytes.

## Product and table ownership

The dependency direction is **SystemT → RadiusT → RadiusCore**. Each product owns its writes and publishes commands or events for the layer above. RadiusCore can run independently. RadiusT can manage subscribers and receive payments without SystemT. The RADIUS packet path reads RadiusCore's compiled state only. If a required rewrite fails, it makes zero access-account or backend lookup calls.

### RadiusCore `rc_`

The table inventory below contains all 19 core tables. The three IP resource tables are installed by migration 0003. The AAA principal is `rc_access_accounts`. Its identities, credentials, effective access flag, and technical reply profile live in the core. An access account can represent PPPoE, IPoE, hotspot, or an independently scoped device administrator. Contact details, packages, customer payment records, and financial balances belong to the product extensions.

### RadiusT `rt_`

These are the proposed RadiusT tables. Their names and ownership are defined here. Their DDL follows in the RadiusT project.

| Table | Authoritative responsibility |
|---|---|
| `rt_subscribers` | Subscriber/customer account number, contact details, and customer-level status. |
| `rt_packages` | Package code, name, availability, and mapping to `rc_service_profiles`. A standalone quoted price can live here if RadiusT needs one. |
| `rt_subscriptions` | A customer's service instance, current package, and business service state. A subscriber can have multiple subscriptions. |
| `rt_subscription_access_accounts` | Links a subscription to its managed `rc_access_accounts`, allowing separate access accounts per service instance and keeping admin accounts outside subscriber management. |
| `rt_payments` | Basic collection record, including receipt/reference, received amount, currency, method, timestamp, and collection or void status. Does not calculate receivables. |
| `rt_service_actions` | Disconnect and reconnect requests, actor/reason, idempotency reference, and execution result. Also tracks package and access provisioning changes that require a retryable core update. |

RadiusT owns the desired service state. RadiusCore owns effective access enforcement and live sessions. A disconnect first applies the access restriction to the core, then attempts to terminate any live sessions through the available NAS mechanism. RadiusT records whether that action is pending, enforced for new access only, or confirmed for live sessions. A reconnect restores the appropriate access policy through RadiusCore. Each action has an idempotency reference and a service revision guard so a late retry cannot undo a newer decision.

### SystemT `st_`

These names describe **logical ownership for new integration work**. Existing SystemT objects are mapped to their current equivalents before any new tables are created. Current SystemT databases already use names such as `core_billing_schedule`, `core_invoice`, and `core_payment`. This design does not migrate those databases or change those names.

| Logical tables | Authoritative responsibility |
|---|---|
| `st_billing_accounts`, `st_billing_schedules`, `st_unbilled_charges` | Billing terms, service charges, installments, one-time charges, tax inputs, and cycle scheduling. |
| `st_invoices`, `st_invoice_lines` | Posted charges, prior-balance carry-forward, and the customer statement reporting basis. Generate the statement view from posted source data. |
| `st_payments`, `st_payment_allocations` | Financial payment lifecycle, posting, deposits, advances, refunds, and allocation to receivables. |
| `st_remittances`, `st_remittance_items` | Cashier batches, accountability, and closeout. |
| `st_gl_accounts`, `st_journal_entries`, `st_journal_lines` | Chart of accounts and balanced journal entries for basic financial accounting. |

When connected to SystemT, each RadiusT receipt can be handed off using its stable source ID. SystemT processes that ID once, owns the resulting financial posting and allocation, and returns its posting reference. A void or reversal follows the same handoff. RadiusT can show the result from SystemT. An operator action triggered by overdue balances goes through RadiusT; SystemT does not write RadiusCore access flags directly.

A commercial package, a technical reply profile, and a billable price are separate records. RadiusT owns package availability and the technical profile association. RadiusCore owns the attributes it sends. SystemT owns the effective billable rate and tax policy when integrated. RADIUS authentication, session events, and octet/packet counters remain in `rc_acct_*`. Financial balances come from SystemT's posted records.

Use the lower product's API or command contract across deployments. When layers share a database, one-way foreign keys can enforce the same ownership. Store external mappings only where needed for an existing database or an external deployment. Keep service identifiers stable after disconnect or closure so session history can still be reconciled.

## Complete core table inventory

The standalone `radiuscore-schema.sql` is migration 0001 and `radiuscore-dictionary-seed.sql` is migration 0002. The project source contains migrations 0003 for IP resources and 0004 for resolved session accounting. FK columns inherit the parent key's type. IDs stay stable across configuration changes and rollbacks.

| Table | Columns and purpose |
|---|---|
| `rc_schema_migrations` | `version integer`, `checksum rc_digest32`, `applied_at timestamptz`. Applied DDL ledger. |
| `rc_config_revisions` | `revision bigint`, `version varchar(64)`, `manifest jsonb`, `committed_by text`, `committed_at timestamptz`. Immutable configuration revisions, unique semantic version. |
| `rc_config_head` | `singleton boolean`, `revision bigint`. Active cluster revision pointer, updated with compare-and-swap. |
| `rc_sites` | `id integer`, `code rc_code`. Administrative site boundary. |
| `rc_nas` | `id integer`, `site_id integer`, `code rc_code`, `enabled boolean`. Durable NAS identity for accounting and policy. Connection selectors and secret references live in the manifest. |
| `rc_access_nodes` | `id integer`, `site_id integer`, `code rc_code`. Site-scoped OLT/access-node identity. Port IDs are provisioned only where used in an identity selector. |
| `rc_attribute_defs` | `id integer`, `wire_path bigint[]`, `name varchar(128)`, `value_type varchar(24)`, `min_octets integer`, `max_octets integer`, `default_value rc_attr_bytes`. Standard, extended, or vendor dictionary entry. |
| `rc_attribute_enum_values` | `attribute_id integer`, `value rc_uint32`, `name varchar(128)`. Numeric enum labels such as Ethernet and Accounting Start. |
| `rc_service_profiles` | `id integer`, `code rc_code`. Reusable RADIUS reply profile. |
| `rc_service_reply_attributes` | `service_profile_id integer`, `ordinal smallint`, `attribute_id integer`, `value rc_attr_bytes`. Ordered values, with a proposed cap of 200 rows per profile. |
| `rc_access_accounts` | `id bigint`, `service_profile_id integer`, `enabled boolean`. Network principal used by authorization and RADIUS session accounting. |
| `rc_access_identities` | `access_account_id bigint`, `site_id integer`, `keyspace rc_code`, `key_version smallint`, `lookup_key rc_digest32`, `selector jsonb`. One row per alias or identity binding. The composite primary key detects collisions within its scope. |
| `rc_credentials` | `access_account_id bigint`, `method smallint`, `material bytea`. One typed verifier, encrypted compatibility secret, or external verifier reference per method. |
| `rc_acct_nas_state` | `nas_id integer`, `epoch bigint`, `last_boundary_at timestamptz`, `last_boundary_key rc_digest32`. Keeps authenticated NAS lifecycle boundaries consistent across cluster writers. |
| `rc_acct_sessions` | `id bigint`, `nas_id integer`, `nas_epoch bigint`, `session_key rc_digest32`, `acct_session_id bytea`, nullable `access_account_id bigint`, `started_at`, `last_seen_at`, `ended_at` as `timestamptz`, `framed_ipv4 inet`, framed and delegated IPv6 prefixes as `cidr`, octets as `rc_uint64`, packets, elapsed seconds, terminate cause as `rc_uint32`. Current and completed session projection. |
| `rc_acct_events` | `id bigint`, `nas_id integer`, nullable `session_id bigint`, `event_key rc_digest32`, `status_type rc_uint32`, `received_at` and nullable `event_at` as `timestamptz`, nullable wire counters and terminate cause, nullable `pending_reason smallint` and `pending_payload bytea`. Idempotent event ledger, including the reconciliation queue. |
| `rc_ip_spaces` | Routing domain identifier and unique code. One space can span NASes and sites. |
| `rc_ip_pools` | Routing space, pool code, native CIDR pool, optional dynamic prefix length, and enabled status. Allocatable ranges cannot overlap inside a routing space. |
| `rc_ip_assignments` | Static or dynamic IPv4/IPv6 host, link subnet, IPv6 delegation, and routed-prefix assignments. Can reserve address ranges. Contains the lease claim key and timing for dynamic assignments. |

The accounting event journal and session projection serve two different reads. Events support audit and reconciliation. The session row supplies fast active-session and current-usage queries. History is initially unpartitioned to keep the unique event key enforceable across retention boundaries. Reassess with production volume. Any later partitioning needs an explicit deduplication strategy across partitions or a globally unique dedup table.

`rc_uint32` uses `bigint` with a checked 0–4,294,967,295 range. `rc_uint64` uses exact `numeric(20,0)` with a checked 0–18,446,744,073,709,551,615 range. Rust converts those types to `u32` and `u64` with range checks. Digests use exactly 32 bytes. `integer` is sufficient for catalog and infrastructure IDs. Counters stay nullable where a missing counter must be distinguished from zero. Use UTC timestamps and index actual access paths. [PostgreSQL integer and numeric types](https://www.postgresql.org/docs/current/datatype-numeric.html) · [PostgreSQL partitioning](https://www.postgresql.org/docs/current/ddl-partitioning.html)

## Attribute dictionary and defaults

The SQL seed contains the exact initial rows and enum examples. Vendor imports extend the same dictionary. Dictionary IDs are runtime-derived dense slots at generation build time, so sparse PostgreSQL IDs cannot create oversized packet arrays. Wire path examples are `[31]` for Calling-Station-Id and `[26,3561,1]` for the DSL Forum Circuit ID. The compiler validates nested wire paths and vendor layouts, including extended attribute encodings. Min/max count value bytes, excluding headers. Fixed-width values use equal bounds.

| Attribute | Expected type | Value bytes | Catalog default |
|---|---|---:|---|
| User-Name, Calling-Station-Id, NAS-Identifier, NAS-Port-Id | UTF-8 text | 1–253 | NULL |
| User-Password | Protected octets | 16–128 in 16-byte blocks | NULL |
| CHAP-Password | Identifier and digest octets | 17 | NULL |
| NAS-IP-Address, Framed-IP-Address | IPv4 address | 4 | NULL |
| NAS-IPv6-Address | IPv6 address | 16 | NULL |
| Framed-IPv6-Prefix, Delegated-IPv6-Prefix | IPv6 prefix encoding | 2–18, validated per codec | NULL |
| Agent-Circuit-Id, Agent-Remote-Id | Opaque value with device-specific parser | 0–63 for the standard seed | NULL |
| Acct-Status-Type, NAS-Port-Type, Service-Type, Framed-Protocol, Acct-Terminate-Cause | Unsigned enum | 4 | NULL |
| Acct-Session-Id, Acct-Multi-Session-Id | Text in the standard dictionary | 1–253 | NULL |
| Acct-Session-Time, octets, gigawords, packet counts, Acct-Delay-Time | Unsigned integer | 4 each | NULL |
| Event-Timestamp | RADIUS date | 4 | NULL |
| Session-Timeout, Idle-Timeout, Acct-Interim-Interval | Unsigned seconds | 4 | NULL |
| State, Class, Proxy-State | Opaque bytes | 1–253 each | NULL |
| Message-Authenticator | Protocol-managed octets | 16 | NULL |

A dictionary default is optional typed value bytes. An explicit policy can apply it when constructing an outbound reply. Service profile values override reply defaults. A missing inbound credential, caller ID, service identifier, or accounting counter remains missing. Set commercial session limits and interim accounting intervals on the service profile or explicit reply policy.

Allow vendor repair by setting a narrowly scoped **received value override** for the known peer and wire path. Structural bounds stay mandatory. A structurally valid oversized DSL Forum VSA can reach the rewrite engine when the peer override allows it. Validate rewritten values against the destination dictionary and envelope on outbound encoding. Internal policy values can separately specify their allowed type and length. This supports devices that send unusual case, extra padding, prefixed hex, or binary data. [IANA RADIUS dictionary](https://www.iana.org/assignments/radius-types) · [RFC 8044 data types](https://www.rfc-editor.org/rfc/rfc8044.html) · [RFC 4679 DSL Forum attributes](https://www.rfc-editor.org/rfc/rfc4679.html) · [RFC 6929 attribute extensions](https://www.rfc-editor.org/rfc/rfc6929.html)

## Request path, identity, and credentials

1. Resolve the configured ingress peer and pin the active generation. Receivers and worker pools scale with process-available CPUs. A packet remains on one worker through decode, rewrite, and local policy. Bound ingress and outbound queues and isolate accounting persistence capacity from authentication.
2. Apply peer-specific integrity policy to original bytes. Decode once, keeping original attribute spans and direct slots for selected values. Resolve the downstream NAS once if a trusted proxy serves several NAS devices. Validate proxy-to-NAS bindings during compilation.
3. Select the flattened plan by resolved slot. Match exact bytes, prefixes, bounded typed patterns, or precompiled regex guards against received values. Use first matching rule per target. Support trim, fixed strip, capture, slice, literal prefix/suffix, ASCII case, byte/int extraction, hex text/bytes, and validated MAC rendering. Buffers and regex scratch are worker-local.
4. Require every lookup-critical output to be valid. A guard miss can advance to the next alternative. A matching rule that cannot render a valid result fails the required rewrite. Ambiguous identity duplicates fail. Apply the validated rewrites together in the working packet before any identity lookup. O(1) plan dispatch is fixed after peer resolution. Parsing and matching still scale with bounded input and candidate count.
5. Select the configured keyspace. Construct a versioned, length-prefixed identity tuple with site scope, username where used, and optional normalized caller MAC, remote ID, circuit ID, NAS/OLT identity, or access port. Encode missing, empty, and populated components distinctly. For no roaming, include the configured OLT and port scope. Roaming keyspaces use explicitly provisioned site aliases or a shared policy scope. Hash the tuple with a keyed digest and use one composite map probe per configured active version. Keep key-rotation lookups outside steady-state or explicitly bound them during transition.
6. Verify credentials separately from identity lookup. PAP uses a salted password verifier. PPP CHAP compatibility requires a protected recoverable shared secret or a compatible external verifier. Empty password is allowed only when the selected method/policy explicitly permits it. Option 82 or MAC-only access needs an explicitly trusted source and enabled identity-based policy. Use separate admin/device-login policies so subscriber network credentials cannot inherit shell access. [RFC 2865 authentication attributes](https://www.rfc-editor.org/rfc/rfc2865.html)
7. Construct reply from the service profile and policy and enforce the packet budget. Keep protocol integrity, state, EAP, and credentials beyond the generic rewrite target list. If outbound proxying is enabled, encode and protect the new packet for the next hop.

Use bounded, nonblocking queues and a separate capacity limit for expensive credential verification. If verifier work becomes an independent CPU pool, dispatch only after rewrite and lookup. Cache invalidation publishes coherent generations. Apply an emergency deny path or fail closed whenever the configured credential/disablement freshness limit is exceeded. Collect per-worker counters and aggregate outside the packet path. Prefer identifier-free metrics.

## IP resources and assignment

RadiusCore owns allocation and route reply encoding. RadiusT provisions assigned resources on the subscriber's access account and may offer them by package. SystemT can charge for the service; the charge has no effect on address uniqueness. Use a shared public routing space across sites that advertise the same public network. Use distinct spaces to permit overlap between isolated VRFs.

| Use | RADIUS reply |
|---|---|
| IPv4 or IPv6 host | `Framed-IP-Address` or `Framed-IPv6-Address` |
| IPv4 host with a link subnet | `Framed-IP-Address` and `Framed-IP-Netmask` |
| IPv6 link prefix | `Framed-IPv6-Prefix` |
| DHCPv6 delegation | `Delegated-IPv6-Prefix` |
| Routed IPv4 block or IPv6 prefix | `Framed-Route` or `Framed-IPv6-Route` |

The database excludes overlapping active assignments and reserves excluded ranges. Rust finds an aligned free prefix by skipping occupied ranges. Dynamic writers lock a pool through selection and claim; repeated claims return the existing assignment. Renewals and expiry releases use the lease identity. Expiry release checks the previously observed expiry and requires proof the NAS/session has stopped using the resource. Enable NAS-owned pool names only on explicitly configured NAS profiles, with the NAS as the sole allocator for that address space. Test installation of route attributes against each NAS and service. Upstream route advertisement is managed by network routing policy.

The implemented SQL accounts for resolved Start, Interim-Update and Stop with a transactional event insert, duplicate handling and session projection. The runtime must resolve the verified NAS epoch and correlate the session, then invoke this transaction. Accounting-On/Off, reconciliation and the durable spool are specified in the architecture and still require runtime wiring.

## Accounting lifecycle

| Packet or condition | Proposed behavior |
|---|---|
| Start | Insert the event and open or attach to the correct session in one transaction. Bind its stable access account ID when available. |
| Interim-Update | Persist the event and update the projection only if it advances the session under that NAS profile's ordering rules. Keep cumulative octet and packet totals. |
| Stop | Persist final totals and terminate the session idempotently. A late interim or start may fill missing fields and cannot reopen a closed session. |
| Accounting-On / Accounting-Off | Record the lifecycle event and update NAS epoch or close the relevant active sessions only when the authenticated event advances that NAS's lifecycle boundary. |
| Missing rewrite, unresolved access account, ambiguous session, or missing correlation data | Make zero identity lookups on rewrite failure. Store typed pending reason and limited, protected reconciliation input. Retry only through reconciliation. |
| Retransmission | Deduplicate before changing the projection. A replay after commit returns the same outcome. A cluster failover still uses the same database uniqueness constraints. |
| PostgreSQL temporarily unavailable | Use a bounded durable encrypted local spool if enabled. The default acknowledgement policy waits for the database commit. An explicit local-spool acknowledgement mode accepts node-loss exposure and acknowledges only after confirmed durable local write. |

Combine gigawords and low octets in Rust without losing the unsigned range. Keep event time and receipt time distinct. Store counters as received and update the projection from the latest credible evidence. Build a stable event key from a guaranteed device event ID when available. Otherwise fingerprint immutable event fields within the NAS namespace, apart from transport fields that change across retries. Document that legacy devices without unique event identity may report indistinguishable events. Compare and measure deduplication on actual NAS samples. A signed opaque class correlation token or a profile-specific session discriminator can disambiguate reused session IDs; a bare session ID should never bind to a different known session. Retain ambiguous packets for reconciliation.

Use transactional event insert and projection update, batch writes, SQL uniqueness for retries, and checked ordering under concurrency. Reconciliation uses row locking and updates the existing event rather than inserting another accounting record. A pending event has reason codes 1 rewrite failed, 2 access account unresolved, 3 session ambiguous, and 4 required correlation missing. Clearing a queue row removes its transient payload. Spool segments are reclaimed after durable downstream confirmation. The in-memory session cache can expire after the configured session timeout plus idle grace. Database, unresolved queue, and spool retention each have separate limits and reporting. [RFC 2866 accounting](https://www.rfc-editor.org/rfc/rfc2866.html) · [RFC 2869 gigawords](https://www.rfc-editor.org/rfc/rfc2869.html) · [RFC 5080 retries](https://www.rfc-editor.org/rfc/rfc5080.html)

## Configuration files

The following is the proposed complete first implementation file set. Fragment files are convenient for authoring and import. The committed PostgreSQL manifest is the runtime source for cluster policy after bootstrap. Missing optional snippets have explicit compiler defaults.

| File | Responsibility |
|---|---|
| `config/radiuscore.toml` | Local node ID, listeners and transports, CPU workers, queues, buffer ceilings, health/metrics socket, local control socket, log targets, state paths, startup safety. |
| `config/postgresql.toml` | Primary endpoint discovery, optional replica endpoints for read-only reports, TLS, connection reference, pool sizes and timeouts, migration and failover settings. |
| `config/cluster.toml` | Discovery interface, multicast group/port, unicast peers or optional broadcast, heartbeat, authenticated membership, load reports, active-role and NAS migration policy. Local discovery values are bootstrap inputs; cluster policy is committed in the manifest. |
| `config/policy.toml` | PPPoE, IPoE Option 82, hotspot and device login stages; authentication and credential modes; site roaming, identity key composition, timeouts, rate limits, service reply, optional proxy and CoA policy. |
| `config/rewrite.toml` | Vendor profiles, device overrides, stage/target, regex guards, ordered alternatives, input repair exceptions, output type and bounds, required output declarations. |
| `config/peers.toml` | Transport-specific peer and trusted proxy bindings, NAS mapping, secret references, vendor profile, per-device overrides, RADIUS integrity requirements. |
| `config/accounting.toml` | Batch size/time, spool quotas, ack mode, session expiry, epoch handling, reconciliation, pending and event retention, counter behavior. |
| `config/dictionary/*.toml` | Reviewed dictionary seed/overlay inputs imported into PostgreSQL, with versioned compatibility validation on activation. |
| `config/security.toml` | Local admin roles, TLS and cluster trust paths, secret providers/references, key IDs, traffic limits, redaction, optional proxy/dynamic authorization limits. Cluster-wide policy is compiled into the manifest. |
| `config/logging.toml` | Structured logging, sampling, redaction and metric labels; node-local. |
| `config/credentials.d/` | Restricted runtime-mounted credentials and certificates. Versioned files hold references only. |
| `deploy/systemd/radiuscore.service` | Hardened service, inherited credentials, restart and watchdog policy, readiness notification, explicit stop behavior. |

Each PostgreSQL peer row is referenced by durable NAS ID in the manifest. At `commit`, validate scopes, method compatibility, duplicates, regex and length limits, secret availability by reference, and catalog compatibility. Publish an immutable runtime generation after local validation. A configuration rollback recompiles a prior manifest against current operational data and references. Persist the rollback as a new commit so its history is explicit. Revisions reference stable inventory IDs. Keep referenced dictionary definitions immutable in place or introduce a new version before changing their semantics. Avoid retiring operational dependencies still needed for rollback.

Cluster nodes advertise health, CPU pressure, and measured latency over authenticated membership. Discovery proposes peers; explicitly configured trust admits them. Use PostgreSQL coordination for cluster-wide configuration writes and failover fencing for management actions. Active-active packet handling can accept across several nodes. A NAS can move only when the NAS configuration, load balancer, or operator provides the steering mechanism. A heartbeat alone cannot redirect NAS traffic. For dynamic NAS control, bind authenticated peer and session scope and audit the action. [RFC 5176 dynamic authorization](https://www.rfc-editor.org/rfc/rfc5176.html)

## Proposed Rust repository

```text
radiuscore/
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  .cargo/config.toml
  src/
    main.rs
    lib.rs
    cli/{mod.rs,config_shell.rs,commands.rs}
    config/{mod.rs,load.rs,compile.rs,revision.rs}
    transport/{mod.rs,udp.rs,tls.rs,workers.rs}
    radius/{mod.rs,packet.rs,dictionary.rs,integrity.rs,proxy.rs}
    rewrite/{mod.rs,pattern.rs,regex.rs,render.rs}
    identity/{mod.rs,key.rs,cache.rs}
    aaa/{mod.rs,authn.rs,authz.rs,reply.rs,coa.rs}
    accounting/{mod.rs,event.rs,session.rs,spool.rs,writer.rs,reconcile.rs}
    store/{mod.rs,postgres.rs,migrate.rs,cache_sync.rs}
    cluster/{mod.rs,discovery.rs,membership.rs,placement.rs}
    control/{mod.rs,server.rs,supervisor.rs,metrics.rs}
  config/{radiuscore,postgresql,cluster,policy,rewrite,peers,accounting,security,logging}.toml
  config/dictionary/
  migrations/{0001_schema,0002_dictionary,0003_ipam,0004_accounting}.sql
  tests/{protocol,rewrite,postgres,accounting,cluster,interop}.rs
  tests/fixtures/{packets,nas,rewrite,accounting}/
  benches/{rewrite,packet_pipeline,accounting_batch}.rs
  fuzz/fuzz_targets/{radius_decode,rewrite_compile}.rs
  deploy/systemd/radiuscore.service
  deploy/postgresql/{roles.sql,health-check.sql}
  scripts/{install.sh,db-bootstrap.sh,backup.sh,restore.sh,smoke-test.sh,bench.sh}
  .github/workflows/{ci,release}.yml
  docs/{operations,configuration,packet-processing}.md
```

## Rust function inventory

Names are proposed public entry points and core module functions. I/O adapters own PostgreSQL, sockets and secrets; packet-facing modules stay independently testable. Small private helpers follow implementation needs.

| Module | Functions and responsibilities |
|---|---|
| `cli`, `control` | `parse_command`, `run_command`, `open_config_session`, `show_running_config`, `show_candidate_config`, `edit_candidate`, `set_autocommit`, `commit_candidate`, `list_revisions`, `diff_revision`, `restore_revision`, `prune_revisions`, `run_rewrite_test`, `serve_control_socket`, `authorize_command`, `reload_runtime`, `request_shutdown`, `status`. |
| `config` | `load_bootstrap`, `load_candidate`, `resolve_secret_refs`, `validate_manifest`, `compile_generation`, `publish_generation`, `pin_generation`, `compare_and_swap_head`, `recover_last_good_generation`. |
| `transport` | `bind_listeners`, `accept_tls_peer`, `resolve_ingress_peer`, `receive_batch`, `dispatch_packet`, `run_worker`, `send_reply`, `drain_workers`. |
| `radius` | `decode_header`, `validate_ingress_integrity`, `decode_attributes`, `resolve_downstream_nas`, `attribute`, `replace_attribute`, `validate_attribute_value`, `load_dictionary`, `import_dictionary`, `decode_wire_path`, `encode_reply`, `protect_outbound`, `classify_retransmission`, `proxy_request`, `accept_proxy_reply`. |
| `rewrite` | `compile_plan`, `compile_matcher`, `compile_template`, `plan_for_slot`, `evaluate_guard`, `match_input`, `capture_fields`, `render_output`, `encode_output`, `validate_output`, `apply_required_rewrites`, `describe_failure`. |
| `identity` | `select_keyspace`, `encode_identity_tuple`, `derive_lookup_key`, `resolve_access_account`, `provision_identity`, `validate_alias`, `reindex_keyspace`, `publish_identity_index`. |
| `aaa` | `handle_access_request`, `verify_pap`, `verify_chap`, `verify_external`, `authorize_trusted_line`, `authorize_device_login`, `enforce_site_roaming`, `build_service_reply`, `issue_correlation_token`, `verify_correlation_token`, `build_access_reject`, `send_coa`, `send_disconnect`, `handle_dynamic_authorization_reply`. |
| `accounting` | `handle_accounting_request`, `parse_accounting_event`, `combine_octet_counters`, `make_event_key`, `resolve_session_key`, `classify_event_order`, `advance_nas_epoch`, `apply_start`, `apply_interim`, `apply_stop`, `expire_session_cache`, `append_spool`, `sync_spool`, `replay_spool`, `persist_event_batch`, `queue_reconciliation`, `resolve_pending_event`, `purge_expired_events`, `build_accounting_response`. |
| `store` | `connect_primary`, `check_write_primary`, `run_migrations`, `read_config_head`, `insert_config_revision`, `load_catalogs`, `load_hot_access_accounts`, `refresh_hot_state`, `insert_event_once`, `lock_session`, `upsert_session_projection`, `take_pending_events`, `check_lag`. |
| `cluster`, `supervisor` | `start_discovery`, `authenticate_peer`, `publish_heartbeat`, `update_membership`, `sample_node_load`, `recommend_nas_placement`, `apply_fenced_migration`, `observe_cluster_revision`, `run_watchdog`, `report_ready`, `report_degraded`, `restart_failed_worker`, `flush_before_shutdown`, `export_metrics`. |

## Binaries, commands and deployment scripts

| Program or command | Role |
|---|---|
| `radiuscore run` | Production daemon supervisable by systemd. Worker failure triggers a health event and restart or degraded operation. |
| `radiuscore config` | Cisco-style interactive shell. Supports `configure terminal`, `show running-config`, `show candidate-config`, `autocommit`, `commit`, `rollback list`, `rollback list detail`, `rollback list 10`, `rollback 3.4.2`, `rollback latest`, and `rollback clear`. Clear is an authenticated prune operation protecting active and recovery revisions. |
| `radiuscore status`, `radiuscore stop` | Readiness, runtime generation, worker and spool status; explicit graceful shutdown. |
| `radiuscore reload`, `radiuscore reload no-load` | Proposed semantics are rebuild from the active committed version and refresh local runtime controls while retaining the current compiled generation, respectively. |
| `radiuscore cluster status` | Members, revision compatibility, load/latency, health, steering status. |
| `radiuscore config check`, `radiuscore rewrite test` | Offline compile validation and sample packet/attribute tracing with sensitive values redacted by default. |
| `radiuscore db migrate`, `radiuscore dictionary import` | Ordered, checksum-checked schema updates and reviewed dictionary loading. |
| `radiuscore accounting pending`, `radiuscore accounting reconcile`, `radiuscore accounting spool status` | Operator recovery and visibility. |
| `radiuscore coa disconnect` | Authorized dynamic session disconnect when enabled. |
| `install.sh` | Install the one production executable, template configuration and service with operator-set secrets. |
| `db-bootstrap.sh` | Create restricted migrator and runtime privileges, then run the migrations. |
| `backup.sh`, `restore.sh` | PostgreSQL and configuration recovery workflow, with separate credential/key recovery and an isolated restore check. |
| `smoke-test.sh`, `bench.sh` | Exercise working PPPoE/Option 82 and accounting flows, then measure packet throughput, latency, CPU and queue drops. |
| Cargo test, bench and fuzz executables | Development and CI only. A lab RADIUS traffic client and NAS fixtures feed interop tests. |

The migration runner creates its ledger and applies each migration/checksum in a single transaction under an advisory lock. The first migration is the ledger bootstrap; for a standalone dry run use the migration files in order with `psql -X --single-transaction -v ON_ERROR_STOP=1`. Production startup validates schema compatibility. No automatic restore or destructive retention runs from startup.

## Verified implementation snapshot

The source package includes a Rust release build, guarded rewrite and RADIUS/Option 82 fixtures, IPAM allocation and reply encoding, and executable migrations 0001–0004. PostgreSQL 17.12 passed the migration, concurrent allocation, lease retry and session-accounting checks. An isolated FreeRADIUS 3.2.7 test captured a RADIUS request and checked an IPv4 host, routed IPv4 block and delegated IPv6 prefix in FreeRADIUS's reply. RadiusCore decoded the captured Option 82 attributes. The public examples and test procedure are under `docs/option82-interop.md`.

RadiusCore's own verified network service, database runtime, cluster deployment, and target NAS sign-off are pending. The reference FreeRADIUS exchange does not certify RadiusCore's UDP/TLS or end-to-end RADIUS authentication.

## Acceptance checks before deployment

- Round-trip original wire bytes, malformed fields, VSA boundary handling, typed defaults, and MAC/circuit/remote ID rewrites. Show zero lookup calls on every required rewrite failure.
- Verify one direct plan access after peer resolution, no repeated attribute scans, and no per-packet rewrite compilation. Measure one worker and process-available CPUs, p50/p99 at fixed offered load, allocation counts, queue drops, and behavior during reloads and accounting spikes.
- Exercise actual MikroTik PPPoE, hotspot and device-login profiles, and Option 82 from ZTE and VSOL through the configured NAS. Validate vendor exceptions against capture fixtures. Add Cisco and Huawei administrator login policy fixtures and verify role isolation.
- Verify empty-password policy, tri-state identity inputs, case changes and alias collisions, dual-stack replies, CoA authorization, peer and secret rotation, rollout rollback, and stale-cache behavior.
- Replay accounting duplicates, out-of-order packets, NAS restarts, reused session IDs, counters at unsigned boundaries, PostgreSQL failover, spool restart, and node loss in each acknowledgement mode.
- Verify graceful stop, supervised restart, health signals, cluster revision convergence, and steering under load. Keep the client platform integration behind the local control API until its authorization contract is defined.

**Open production choices:** Measure hardware before choosing packet-per-second targets, worker caps or disk quotas. Confirm retention periods, peer TLS compatibility, secret provider, and whether local-spool acknowledgement is acceptable. The baseline uses a conservative database-commit acknowledgement policy. The starter dictionary plus migration 0003 is the reviewed deployment subset and requires vendor imports for additional real devices.

## Starter configuration templates

These fragments specify the proposed file format. Sample endpoints, quotas and timing values are starting points for a lab. Review operational values before deployment.

### config/radiuscore.toml

```toml
node_id = "rc-node-01"
state_dir = "/var/lib/radiuscore"
control_socket = "/run/radiuscore/control.sock"
workers = "auto"
worker_queue_capacity = 1024

[[listeners]]
transport = "udp"
bind = "127.0.0.1:1812"
stage = "access"

[[listeners]]
transport = "udp"
bind = "127.0.0.1:1813"
stage = "accounting"
```

### config/postgresql.toml

```toml
primary_endpoints = ["db.example.invalid:5432"]
database = "radiuscore"
user = "radiuscore_runtime"
credential_ref = "file:db-password"
tls_mode = "verify-full"
pool_max = 16
connect_timeout_ms = 3000
```

### config/cluster.toml

```toml
enabled = false
discovery_mode = "multicast"
discovery_interface = "eth0"
discovery_group = "224.0.0.224"
discovery_port = 41812
heartbeat_ms = 1000
membership_timeout_ms = 5000
placement_mode = "recommend"
```

### config/peers.toml

```toml
[[peers]]
nas = "lab-nas"
transport = "udp"
source = "192.0.2.60/32"
shared_secret_ref = "file:lab-nas-secret"
message_authenticator = "required"
rewrite_profile = "lab-option82"
```

### config/policy.toml

```toml
[[access_policies]]
name = "lab-ipoe"
nas = "lab-nas"
mode = "ipoe-option82"
identity_scope = "site"
identity_fields = ["site", "access_node", "Agent-Circuit-Id", "Agent-Remote-Id"]
keyspace = "ipoe"
key_version = 1
roaming = false
trusted_line_auth = true
```

### config/rewrite.toml

```toml
[rewrite]
max_rules_per_target = 16
max_regex_source_bytes = 2048

[[profiles]]
name = "lab-option82"

[[profiles.received_overrides]]
attribute = "Agent-Circuit-Id"
max_octets = 128

[[profiles.rules]]
stages = ["access", "accounting"]
target = "Agent-Circuit-Id"
required = true
when_attribute = "Calling-Station-Id"
when_regex = '^(?i:00:11:22(?::[0-9a-f]{2}){3})$'
input_type = "text"
input_pattern = "CID={id:hex};END"
output_type = "text"
output_template = "0x{id:upper}"

[[profiles.rules]]
stages = ["access", "accounting"]
target = "Agent-Remote-Id"
required = true
input_type = "text"
input_pattern = "REMOTE-ID={id}"
output_type = "text"
output_template = "{id:upper}"
```

The example expects both IDs and the caller ID on accounting packets. If the NAS omits those values, use the stage-specific policy and session correlation mechanism it can support. A packet whose selected required rewrite fails never reaches subscriber or backend lookup.

### config/accounting.toml

```toml
ack_mode = "postgres-commit"
batch_max_events = 256
batch_wait_ms = 10
spool_enabled = true
spool_max_bytes = 1073741824
spool_key_ref = "file:accounting-spool-key"
session_cache_idle_grace_seconds = 600
pending_retention_days = 14
event_retention_days = 90
```

### config/security.toml

```toml
credentials_dir = "/run/credentials/radiuscore.service"
identity_key_ref = "file:identity-key-v1"
max_authorization_snapshot_age_seconds = 300
redact_binary_payloads = true
```


### config/logging.toml

```toml
level = "info"
format = "json"
redact_attributes = true
packet_trace = false
```

### config/dictionary/example.toml

```toml
[[attributes]]
wire_path = [26, 3561, 1]
name = "Agent-Circuit-Id"
value_type = "octets"
min_octets = 0
max_octets = 63
```

Omit the optional dictionary default for SQL NULL. The import format can use `default_hex` when a typed default is needed. A device-specific input exception belongs in its rewrite profile.
