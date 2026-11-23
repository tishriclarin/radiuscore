-- RadiusCore proposed baseline. Apply to a dedicated database through a
-- migration runner that records revisions in rc_schema_migrations.
-- This file is the body of migration 0001 and must run once.

CREATE DOMAIN rc_uint32 AS bigint CHECK (VALUE BETWEEN 0 AND 4294967295);
CREATE DOMAIN rc_uint64 AS numeric(20,0)
    CHECK (VALUE BETWEEN 0 AND 18446744073709551615);
CREATE DOMAIN rc_digest32 AS bytea CHECK (octet_length(VALUE) = 32);
CREATE DOMAIN rc_code AS varchar(64) CHECK (VALUE ~ '^[A-Za-z0-9][A-Za-z0-9_.-]*$');
CREATE DOMAIN rc_attr_bytes AS bytea CHECK (octet_length(VALUE) <= 65535);

CREATE TABLE rc_schema_migrations (
    version integer PRIMARY KEY CHECK (version > 0),
    checksum rc_digest32 NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE rc_config_revisions (
    revision bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    version varchar(64) NOT NULL UNIQUE,
    manifest jsonb NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
    committed_by text NOT NULL,
    committed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE rc_config_head (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision bigint NOT NULL REFERENCES rc_config_revisions
);

CREATE TABLE rc_sites (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code rc_code NOT NULL UNIQUE
);

CREATE TABLE rc_nas (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    site_id integer NOT NULL REFERENCES rc_sites,
    code rc_code NOT NULL UNIQUE,
    enabled boolean NOT NULL DEFAULT true
);

CREATE TABLE rc_access_nodes (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    site_id integer NOT NULL REFERENCES rc_sites,
    code rc_code NOT NULL,
    UNIQUE (site_id, code)
);

CREATE TABLE rc_attribute_defs (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    wire_path bigint[] NOT NULL UNIQUE CHECK (
        array_ndims(wire_path) = 1 AND array_lower(wire_path, 1) = 1
        AND cardinality(wire_path) BETWEEN 1 AND 16
        AND array_position(wire_path, NULL) IS NULL
        AND wire_path[1] BETWEEN 1 AND 255
    ),
    name varchar(128) NOT NULL UNIQUE CHECK (name <> ''),
    value_type varchar(24) NOT NULL CHECK (value_type IN (
        'text', 'octets', 'integer', 'enum', 'integer64', 'date',
        'ipv4addr', 'ipv6addr', 'ipv4prefix', 'ipv6prefix',
        'ifid', 'tlv', 'vsa', 'extended', 'long-extended', 'evs'
    )),
    min_octets integer NOT NULL CHECK (min_octets BETWEEN 0 AND 65535),
    max_octets integer NOT NULL CHECK (max_octets BETWEEN min_octets AND 65535),
    -- NULL means no default. Defaults are opt-in and encoded as value bytes.
    default_value rc_attr_bytes,
    CHECK (default_value IS NULL OR octet_length(default_value) BETWEEN min_octets AND max_octets)
);

CREATE TABLE rc_attribute_enum_values (
    attribute_id integer NOT NULL REFERENCES rc_attribute_defs,
    value rc_uint32 NOT NULL,
    name varchar(128) NOT NULL CHECK (name <> ''),
    PRIMARY KEY (attribute_id, value),
    UNIQUE (attribute_id, name)
);

CREATE TABLE rc_service_profiles (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code rc_code NOT NULL UNIQUE
);

CREATE TABLE rc_service_reply_attributes (
    service_profile_id integer NOT NULL REFERENCES rc_service_profiles ON DELETE CASCADE,
    ordinal smallint NOT NULL CHECK (ordinal BETWEEN 1 AND 200),
    attribute_id integer NOT NULL REFERENCES rc_attribute_defs,
    value rc_attr_bytes NOT NULL,
    PRIMARY KEY (service_profile_id, ordinal)
);

-- Network principals for AAA. Commercial subscriber records are owned by RadiusT.
CREATE TABLE rc_access_accounts (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    service_profile_id integer NOT NULL REFERENCES rc_service_profiles,
    enabled boolean NOT NULL DEFAULT true
);

CREATE TABLE rc_access_identities (
    access_account_id bigint NOT NULL REFERENCES rc_access_accounts,
    site_id integer NOT NULL REFERENCES rc_sites,
    keyspace rc_code NOT NULL,
    key_version smallint NOT NULL CHECK (key_version > 0),
    lookup_key rc_digest32 NOT NULL,
    -- Provisioned identity fields only. Credentials have their own table.
    selector jsonb NOT NULL CHECK (jsonb_typeof(selector) = 'object'),
    PRIMARY KEY (site_id, keyspace, key_version, lookup_key)
);
CREATE INDEX rc_access_identities_account_idx ON rc_access_identities (access_account_id);

CREATE TABLE rc_credentials (
    access_account_id bigint NOT NULL REFERENCES rc_access_accounts ON DELETE CASCADE,
    method smallint NOT NULL CHECK (method IN (1, 2, 3)),
    material bytea NOT NULL CHECK (octet_length(material) > 0),
    PRIMARY KEY (access_account_id, method)
);
COMMENT ON TABLE rc_credentials IS
    'Method 1 PAP verifier, 2 encrypted CHAP secret, 3 external verifier reference.';

CREATE TABLE rc_acct_nas_state (
    nas_id integer PRIMARY KEY REFERENCES rc_nas,
    epoch bigint NOT NULL DEFAULT 0 CHECK (epoch >= 0),
    last_boundary_at timestamptz,
    last_boundary_key rc_digest32
);

CREATE TABLE rc_acct_sessions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    nas_id integer NOT NULL REFERENCES rc_nas,
    nas_epoch bigint NOT NULL CHECK (nas_epoch >= 0),
    session_key rc_digest32 NOT NULL,
    acct_session_id bytea NOT NULL CHECK (octet_length(acct_session_id) BETWEEN 1 AND 253),
    access_account_id bigint REFERENCES rc_access_accounts,
    started_at timestamptz,
    last_seen_at timestamptz NOT NULL,
    ended_at timestamptz,
    framed_ipv4 inet CHECK (family(framed_ipv4) = 4 AND masklen(framed_ipv4) = 32),
    framed_ipv6_prefix cidr CHECK (family(framed_ipv6_prefix) = 6),
    delegated_ipv6_prefix cidr CHECK (family(delegated_ipv6_prefix) = 6),
    input_octets rc_uint64,
    output_octets rc_uint64,
    input_packets rc_uint32,
    output_packets rc_uint32,
    session_seconds rc_uint32,
    terminate_cause rc_uint32,
    UNIQUE (nas_id, nas_epoch, session_key),
    UNIQUE (nas_id, id),
    CHECK (started_at IS NULL OR ended_at IS NULL OR ended_at >= started_at)
);
CREATE INDEX rc_acct_sessions_active_account_idx
    ON rc_acct_sessions (access_account_id, last_seen_at) WHERE ended_at IS NULL;
CREATE INDEX rc_acct_sessions_active_nas_idx
    ON rc_acct_sessions (nas_id, last_seen_at) WHERE ended_at IS NULL;

CREATE TABLE rc_acct_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    nas_id integer NOT NULL REFERENCES rc_nas,
    session_id bigint,
    event_key rc_digest32 NOT NULL,
    status_type rc_uint32 NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    event_at timestamptz,
    input_octets rc_uint64,
    output_octets rc_uint64,
    input_packets rc_uint32,
    output_packets rc_uint32,
    session_seconds rc_uint32,
    terminate_cause rc_uint32,
    pending_reason smallint CHECK (pending_reason BETWEEN 1 AND 4),
    pending_payload bytea CHECK (octet_length(pending_payload) <= 131072),
    UNIQUE (nas_id, event_key),
    FOREIGN KEY (nas_id, session_id) REFERENCES rc_acct_sessions (nas_id, id),
    CHECK ((pending_reason IS NULL) = (pending_payload IS NULL)),
    CHECK (pending_reason IS NULL OR session_id IS NULL)
);
CREATE INDEX rc_acct_events_session_idx ON rc_acct_events (session_id, event_at, id)
    WHERE session_id IS NOT NULL;
CREATE INDEX rc_acct_events_pending_idx ON rc_acct_events (received_at, id)
    WHERE pending_reason IS NOT NULL;
CREATE INDEX rc_acct_events_retention_idx ON rc_acct_events USING brin (received_at);

