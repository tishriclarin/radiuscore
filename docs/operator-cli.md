# RadiusCore operator CLI

The executable is `radiuscore`. This guide covers the commands in the current development build and the planned interface for operating a live RadiusCore service.

## Build and check the executable

Build from the repository root with Rust 1.80 or later and the operating system's OpenSSL development libraries. The transport tests also use the OpenSSL CLI.

```sh
cargo build --locked --release
./target/release/radiuscore --version
./target/release/radiuscore --help
```

The examples below assume the binary is on `PATH`. Substitute `./target/release/radiuscore` when running from the source checkout.

## Current command reference

Use `radiuscore help <group> [<command>]` or append `--help` to a group or command. Running `radiuscore` with no arguments prints general help. Commands return exit code 0 on success and a nonzero exit code after errors. Diagnostics go to standard error.

| Command | Purpose |
|---|---|
| `radiuscore --help` or `radiuscore -h` | List command groups and help syntax. |
| `radiuscore --version` or `radiuscore -V` | Print the version embedded at build time. |
| `radiuscore ip parse <network/cidr>` | Validate and print a canonical IPv4 or IPv6 network. The address must already have host bits cleared. |
| `radiuscore option82 normalize-log <value>` | Convert prefixed hex or a supported escaped log value to lowercase, `0x`-prefixed hex. Quote shell arguments with spaces or backslashes. |
| `radiuscore option82 decode-dhcp <hex-option-payload>` | Decode the DHCP Option 82 payload beginning with its first suboption. Remove the outer DHCP option code and length first. |
| `radiuscore option82 decode-radius <binary-datagram>` | Read a file containing one raw RADIUS datagram and print the recognized DSL Forum circuit and remote IDs. |
| `radiuscore radsec probe <IP:port> <server-name> <ca.pem> <cert.pem> <key.pem> <sha256-pin>` | Open a mutual TLS connection and verify the peer's CA chain, server name and leaf certificate pin. |

### IP and Option 82 examples

```console
$ radiuscore ip parse 192.0.2.16/28
192.0.2.16/28
$ radiuscore ip parse 2001:db8::/48
2001:db8::/48
$ radiuscore option82 normalize-log 0X4142
0x4142
$ radiuscore option82 decode-dhcp 02074d595f41505f310106500000020005
Agent-Circuit-Id=0x500000020005
Agent-Remote-Id=0x4d595f41505f31
$ radiuscore option82 decode-radius request.bin
```

`normalize-log` accepts byte pairs after `0x` or `0X`. In log-text mode it accepts UTF-8 text plus backslash escapes for backslash, double quote, newline, carriage return, tab, and three-digit octal byte values. For `decode-dhcp`, hex may be supplied with or without the prefix. Use a raw RADIUS datagram for `decode-radius`; extract the RADIUS payload first if you have a packet capture. These decoders inspect attributes for diagnostics. They do not authenticate the source, authorize a subscriber, modify a session, or allocate an address. A successful decode with no matching fields can produce no output.

### RadSec peer probe

```sh
radiuscore radsec probe 192.0.2.10:2083 radsec.example.net \
  ca.pem client-cert.pem client-key.pem "$server_pin"
```

Set `server_pin` to the server leaf certificate's 64-digit hexadecimal SHA-256 fingerprint without colons. Obtain it through a trusted channel. Use the server name in the certificate, even when dialing an IP address. Enclose an IPv6 address in brackets, such as `[2001:db8::10]:2083`. The client key is read from the supplied file. The probe reports the verified hostname and certificate fingerprint. It opens a TLS session without sending a RADIUS request. See [transport security](transport-security.md) for scope and limits.

## Planned management CLI

The following commands are the design for a live service and are unavailable in the current diagnostic build. The standalone shell starts with `radiuscore config`. An external management CLI can embed the same command tree and call the authenticated control interface. Both entry points use the same authorization and commit logic.

| Context | Planned command | Operation |
|---|---|---|
| Host shell | `radiuscore status` | Show readiness, active config revision, workers, database and accounting state. |
| Host shell | `radiuscore cluster status` | Show cluster membership and health. |
| Host shell | `radiuscore reload` | Rebuild the active runtime from the committed configuration. |
| Host shell | `radiuscore reload no-load` | Refresh local runtime controls while retaining the current compiled configuration. |
| Host shell | `radiuscore stop` | Request authorized, graceful shutdown. |
| Management shell | `?` | List commands allowed in the current mode and show arguments for partially entered commands. |
| Management shell | `configure terminal` | Enter candidate configuration mode. |
| Management shell | `show running-config` | Show the committed, active configuration with secrets redacted. |
| Management shell | `show candidate-config` | Review changes staged for commit. |
| Configuration mode | `autocommit` | Enable automatic validation and commit of each accepted change. |
| Configuration mode | `commit` | Validate and activate the candidate as a semantic-versioned snapshot. A failed commit keeps the previous active snapshot. |
| Management shell | `rollback list` | List numbered versions and short configuration summaries. |
| Management shell | `rollback list detail` or `rollback list 10` | Compare saved versions, or the specified numbered entry, with the running configuration. |
| Management shell | `rollback 3.4.2` or `rollback latest` | Recover a saved configuration as a new audited commit. |
| Management shell | `rollback clear` | Prune eligible snapshots with authorization while preserving the active and recovery revisions. |

When the runtime is available, review staged changes before committing peer, site or authorization updates. A reload or configuration rollback does not undo PostgreSQL migrations or remove recorded accounting events.

## Planned site and access policy

| Policy | Operators provision | Expected access result |
|---|---|---|
| Site-scoped access | A stable site ID and its NAS or OLT group | Apply the site's device policies and subscriber identity rules. |
| Roaming | An explicit list of allowed sites or devices | Resolve the subscriber from an allowed location. Deny access when the peer or site falls outside the policy. |
| GPON virtual-port lock | The authorized site, access node or OLT, and vendor-normalized virtual port | Authenticate only when that verified path matches the provisioned binding. Deny when a required component is missing, invalid, ambiguous, or different. |

A virtual-port number by itself is not globally unique. Use the trusted site and OLT context along with the normalized port. Field mapping depends on the device profile, for example a profile may extract the port from an Option 82 circuit ID. See the [developer guide](developer-guide.md) for policy invariants.

## Planned live logs

The control interface is intended to stream compact, readable events with timestamp, severity, component, site or peer, correlation ID and reason when applicable. An optional structured output will support log collectors. Apply a live regex to the redacted display. Filters should affect only the current viewer and preserve stored log collection, audit records and packet processing.

Illustrative command syntax, pending runtime integration:

```sh
radiuscore logs --follow --regex 'auth|radsec|accounting'
```

Treat filter strings as operator input. Bound regex compilation, event length and streaming queues. Redact passwords, shared secrets, private keys and subscriber identifiers before display and filtering.

## Troubleshooting

| Symptom | Check and next action |
|---|---|
| `radiuscore: command not found` | Build the binary and run it from `target/release`, or put it on `PATH`. Use `radiuscore --version` to confirm which binary is running. |
| `unknown arguments` | Compare the invocation with `radiuscore --help` or the command's `--help`. The development build only accepts the commands in the current reference. |
| IP parsing fails | Check the prefix length and remove host bits. Use a host prefix such as `192.0.2.17/32` if the host itself is the intended address. |
| Hex or escape decode fails | Check for paired hex digits and shell quoting. Strip any log label or surrounding punctuation before normalizing the value. |
| DHCP TLV invalid or truncated | Confirm the capture includes the entire Option 82 value and omit the outer DHCP option header. |
| RADIUS diagnostic fails or prints no IDs | Check that the file is one untruncated RADIUS payload up to 4096 bytes. Confirm the packet contains the expected DSL Forum VSA, rather than an attribute using a different vendor mapping. |
| RadSec connection refused or timed out | Check the dialed IP and port, peer listener and network path. The probe uses a five-second TCP connect timeout and a separate five-second TLS handshake budget. |
| RadSec certificate rejected | Check both peers' trust bundles, certificate expiry and clock, client certificate and key pair, server certificate name, and the expected leaf fingerprint. See [transport security](transport-security.md). |
| PostgreSQL regression fails | Run the [database tests](developer-guide.md#build-and-test) against a new disposable database. Check connection permissions and extension availability. Keep production data out of test fixtures. |
| Need help reproducing an Option 82 issue | Use the [interop guide](option82-interop.md). Include the NAS firmware, expected site and access-node scope, and sanitized attribute examples. |

When reporting an issue, include the exact command, error, build version or commit and relevant PostgreSQL, OpenSSL or device firmware versions. Remove secrets and subscriber data. See [Support](../SUPPORT.md).
