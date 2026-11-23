# Option 82 interop

## Public log fixtures

- RouterOS community output includes a binary six-octet Circuit ID and a Remote ID whose bytes decode as ASCII. The DHCP option payload in the test is assembled from those values.
- An older FreeRADIUS debug report displays octal and named control-character escapes. Its supplied Circuit ID display decodes to five octets, while the author reports a six-octet expectation. The regression test preserves the five supplied bytes and diagnoses the discrepancy. A capture is needed to establish whether the original packet contained the extra byte.
- The RADIUS test wraps public values in a synthetic DSL Forum VSA. The internet sources are field reports and logs. Treat firmware-specific interpretation as unconfirmed until device testing.

[RouterOS Field report](https://forum.mikrotik.com/t/dhcp-option-82-dhcp-snooping/108132) · [Legacy FreeRADIUS debug report](https://forum.mikrotik.com/t/mikrotik-dhcp-server-option82-freeradius-incorrect-format/42682) · [RouterOS Option 82 behavior](https://manual.mikrotik.com/docs/bridging-and-switching/)

## Lab comparison

1. Capture DHCP Discover/Request at the configured relay and the corresponding RADIUS Access-Request at the NAS. Capture Accounting-Start and Interim/Stop if applicable. Retain firmware, service type and the exact configured Option 82 template. Redact credentials and unrelated identifying values.
2. Validate Option 82 bytes and lengths with Wireshark or a decoded NAS capture, then compare with FreeRADIUS debug. Keep captured DHCP TLVs, RADIUS VSA bytes and display text separately.
3. Create the per-NAS match/rewrite cases from observed encodings. Exercise missing IDs, bad output, duplicates, third-party vendor IDs and ordering. Check that every required failure invokes zero lookup calls.
4. Against a lab NAS, verify an IPv4 host, an IPv6 host, a delegated IPv6 prefix, a routed IPv4 block, and an IPv6 routed prefix where supported. Check installed routes, reachability, retransmissions, disconnect/reconnect, and eventual release.

## Reproduce the FreeRADIUS comparison

Build the release executable and run `python3 tests/freeradius_lab.py` with FreeRADIUS 3.2 and radclient installed. The harness starts a temporary loopback FreeRADIUS instance, validates the resulting IPv4, routed IPv4 block and delegated IPv6 prefix attributes, and passes the captured request bytes through RadiusCore's diagnostic decoder. An isolated FreeRADIUS 3.2.7 run passed. The FreeRADIUS server supplied the live Access-Accept in this test.

Set `FREERADIUS_BIN`, `RADCLIENT_BIN`, or `RADIUS_DICTIONARY_DIR` for nonstandard installations.
