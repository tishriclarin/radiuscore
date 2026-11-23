-- Proposed starter dictionary for the base schema. Run once as migration 0002.
-- Min/max are value octets. All seed definitions inherit NULL defaults.
INSERT INTO rc_attribute_defs (wire_path, name, value_type, min_octets, max_octets)
VALUES
    (ARRAY[1], 'User-Name', 'text', 1, 253),
    (ARRAY[2], 'User-Password', 'octets', 16, 128),
    (ARRAY[3], 'CHAP-Password', 'octets', 17, 17),
    (ARRAY[4], 'NAS-IP-Address', 'ipv4addr', 4, 4),
    (ARRAY[5], 'NAS-Port', 'integer', 4, 4),
    (ARRAY[6], 'Service-Type', 'enum', 4, 4),
    (ARRAY[7], 'Framed-Protocol', 'enum', 4, 4),
    (ARRAY[8], 'Framed-IP-Address', 'ipv4addr', 4, 4),
    (ARRAY[18], 'Reply-Message', 'text', 1, 253),
    (ARRAY[24], 'State', 'octets', 1, 253),
    (ARRAY[25], 'Class', 'octets', 1, 253),
    (ARRAY[26], 'Vendor-Specific', 'vsa', 5, 253),
    (ARRAY[27], 'Session-Timeout', 'integer', 4, 4),
    (ARRAY[28], 'Idle-Timeout', 'integer', 4, 4),
    (ARRAY[30], 'Called-Station-Id', 'text', 1, 253),
    (ARRAY[31], 'Calling-Station-Id', 'text', 1, 253),
    (ARRAY[32], 'NAS-Identifier', 'text', 1, 253),
    (ARRAY[33], 'Proxy-State', 'octets', 1, 253),
    (ARRAY[40], 'Acct-Status-Type', 'enum', 4, 4),
    (ARRAY[41], 'Acct-Delay-Time', 'integer', 4, 4),
    (ARRAY[42], 'Acct-Input-Octets', 'integer', 4, 4),
    (ARRAY[43], 'Acct-Output-Octets', 'integer', 4, 4),
    (ARRAY[44], 'Acct-Session-Id', 'text', 1, 253),
    (ARRAY[45], 'Acct-Authentic', 'enum', 4, 4),
    (ARRAY[46], 'Acct-Session-Time', 'integer', 4, 4),
    (ARRAY[47], 'Acct-Input-Packets', 'integer', 4, 4),
    (ARRAY[48], 'Acct-Output-Packets', 'integer', 4, 4),
    (ARRAY[49], 'Acct-Terminate-Cause', 'enum', 4, 4),
    (ARRAY[50], 'Acct-Multi-Session-Id', 'text', 1, 253),
    (ARRAY[52], 'Acct-Input-Gigawords', 'integer', 4, 4),
    (ARRAY[53], 'Acct-Output-Gigawords', 'integer', 4, 4),
    (ARRAY[55], 'Event-Timestamp', 'date', 4, 4),
    (ARRAY[60], 'CHAP-Challenge', 'octets', 5, 253),
    (ARRAY[61], 'NAS-Port-Type', 'enum', 4, 4),
    (ARRAY[79], 'EAP-Message', 'octets', 1, 253),
    (ARRAY[80], 'Message-Authenticator', 'octets', 16, 16),
    (ARRAY[85], 'Acct-Interim-Interval', 'integer', 4, 4),
    (ARRAY[87], 'NAS-Port-Id', 'text', 1, 253),
    (ARRAY[95], 'NAS-IPv6-Address', 'ipv6addr', 16, 16),
    (ARRAY[97], 'Framed-IPv6-Prefix', 'ipv6prefix', 2, 18),
    (ARRAY[123], 'Delegated-IPv6-Prefix', 'ipv6prefix', 2, 18),
    (ARRAY[101], 'Error-Cause', 'enum', 4, 4),
    (ARRAY[26,3561,1], 'Agent-Circuit-Id', 'octets', 0, 63),
    (ARRAY[26,3561,2], 'Agent-Remote-Id', 'octets', 0, 63);

INSERT INTO rc_attribute_enum_values (attribute_id, value, name)
SELECT d.id, e.value, e.name
FROM (VALUES
    ('Acct-Status-Type', 1, 'Start'),
    ('Acct-Status-Type', 2, 'Stop'),
    ('Acct-Status-Type', 3, 'Interim-Update'),
    ('Acct-Status-Type', 7, 'Accounting-On'),
    ('Acct-Status-Type', 8, 'Accounting-Off'),
    ('NAS-Port-Type', 15, 'Ethernet'),
    ('Service-Type', 1, 'Login-User'),
    ('Service-Type', 2, 'Framed-User'),
    ('Service-Type', 6, 'Administrative-User'),
    ('Framed-Protocol', 1, 'PPP')
) AS e(attribute_name, value, name)
JOIN rc_attribute_defs d ON d.name = e.attribute_name;
