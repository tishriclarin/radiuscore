# RADIUS transport security

## Implemented and tested in the Rust transport library

- RadSec client and server connection primitives use mutual TLS with a minimum of TLS 1.2. Each connection verifies the peer certificate against an explicit CA bundle and a SHA-256 certificate allowlist. Clients verify the expected server name in the certificate. The server exposes the verified pin for NAS policy selection. Rotate certificates by allowing the current and next pins during a defined overlap.
- RADIUS frames use the embedded length field. Handshakes, reads and writes have absolute deadlines. A partially read or written frame closes the usable session, and the reader accepts packets up to 4096 bytes. There is no plaintext fallback.
- The strict legacy UDP Access-Request verifier requires exactly one valid `Message-Authenticator`. It rejects missing, duplicate, malformed, or altered values before returning a verified request.
- Baseline RFC 6614 RadSec uses the fixed RADIUS secret `radsec` inside the authenticated TLS channel. A present legacy Access-Request `Message-Authenticator` is verified, and it is required with EAP. Use the RadSec acceptance API with byte-level integrity checks for Access-Requests. A raw TLS frame establishes transport provenance only and should be sent to the validator for that packet type before policy or side effects.

## Probe a peer

Requires Rust 1.80 or later and system OpenSSL. The tests also use the OpenSSL command-line utility to create temporary lab certificates.

```sh
radiuscore radsec probe <IP:port> <server-name> <ca.pem> <client-cert.pem> <client-key.pem> <sha256-pin>
```

Use the certificate's hexadecimal SHA-256 fingerprint without separators. Pin values must be provisioned from a trusted source. The probe opens and verifies a TLS session. It does not send a RADIUS Access-Request or validate that subscriber authentication works.

## Before production deployment

- Wire the RadSec listener and outbound client into the running AAA server. Bind verified peer pins to explicit NAS policies. Limit simultaneous handshakes and connections, and enforce per-peer rate limits and request deadlines.
- Verify accounting and dynamic-authorization authenticators for each enabled transport. Scope CoA and Disconnect requests to authorized peers and sessions, enforce replay protection, and send accounting acknowledgements after the configured durability boundary.
- Complete UDP response signing and peer policy. Keep strict Access-Request verification as the default. Legacy NAS exceptions need an explicit, isolated policy. Never use unauthenticated Proxy-State to select a backend or build a response.
- Add certificate and secret rotation, restricted secret storage, audit events, and key and certificate lifecycle monitoring. Use a deployment-appropriate revocation strategy.
- Test with physical NAS devices, FreeRADIUS and a RadSec proxy. RFC 9765 RADIUS/1.1 requires explicit ALPN negotiation and a different packet format. This transport does not advertise or implement RADIUS/1.1 yet.

[RFC 6614](https://www.rfc-editor.org/rfc/rfc6614.html) · [RFC 3579](https://www.rfc-editor.org/rfc/rfc3579.html) · [RFC 9765](https://www.rfc-editor.org/rfc/rfc9765.html) · [FreeRADIUS BlastRADIUS guidance](https://www.freeradius.org/vul_notifications/2024/07/09/blastradius.html)
