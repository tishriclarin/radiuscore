# Why RadiusCore was built

A typical ISP needs to recognize a subscriber across a defined set of access devices, enforce where that subscriber may connect, assign an address or prefix, and record the session accurately. Different NAS and OLT models can describe the same access path with different attribute names, formats or encodings. RadiusCore exists to make that common RADIUS workflow predictable on the equipment the ISP actually operates.

## Performance with a defined scope

RadiusCore focuses on typical network equipment and the attributes needed for its ISP access policies. A curated device and attribute set makes it possible to validate formats explicitly, compile device-specific rules once, and keep operator configuration smaller. Performance is a design goal for the complete request path. The Rust daemon target uses multiple threads across cores and CPU sockets, with read-optimized, in-memory policy lookups and PostgreSQL transactions where durable state is needed. The project tracks performance against realistic workloads and publishes measurements when verified.

## Site-aware access

A site ID identifies a group of access equipment. Operators can allow roaming within selected sites or devices or bind authentication to a particular GPON virtual port. Trusted peer configuration determines site and equipment scope. Device-specific normalization produces a stable port identity for lookup. Missing or ambiguous required attributes stop the request before subscriber lookup.

## Subscriber lifecycle and durable accounting

RadiusCore gives authentication, address allocation and accounting a shared identity model. PostgreSQL enforces address assignment constraints and transactionally records resolved accounting events. The design buffers and batches accounting writes to avoid turning bursts into a flood of individual disk writes. The acknowledgement policy preserves a clear boundary between received events and durable records. RadiusT manages the subscriber service and SystemT owns billing.

## Simpler operations

Operators get an offline diagnostic CLI today. The target operator experience includes a Cisco-style shell with candidate configuration, commit and rollback. A management system can embed the same commands. The daemon design calls for cleaner, redacted logs, with a live regex filter for operators following a specific peer, site or request.

[Operator CLI and troubleshooting](operator-cli.md) · [Developer guide](developer-guide.md) · [Engineering blueprint](engineering-blueprint.md)
