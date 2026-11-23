CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE rc_ip_spaces (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code rc_code NOT NULL UNIQUE
);

CREATE TABLE rc_ip_pools (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    space_id integer NOT NULL REFERENCES rc_ip_spaces,
    code rc_code NOT NULL,
    network cidr NOT NULL,
    allocation_prefix_len smallint,
    enabled boolean NOT NULL DEFAULT true,
    UNIQUE (space_id, code),
    UNIQUE (space_id, id),
    CHECK (allocation_prefix_len BETWEEN masklen(network) AND
        CASE family(network) WHEN 4 THEN 32 ELSE 128 END),
    EXCLUDE USING gist (space_id WITH =, network inet_ops WITH &&)
);

CREATE TABLE rc_ip_assignments (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    space_id integer NOT NULL,
    pool_id integer NOT NULL,
    access_account_id bigint REFERENCES rc_access_accounts,
    network cidr NOT NULL,
    purpose smallint NOT NULL CHECK (purpose BETWEEN 1 AND 5),
    next_hop inet,
    link_host inet,
    claim_key rc_digest32,
    assigned_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz,
    released_at timestamptz,
    FOREIGN KEY (space_id, pool_id) REFERENCES rc_ip_pools (space_id, id),
    -- Purpose 1 host, 2 link subnet, 3 delegated IPv6, 4 routed, 5 excluded/reserved.
    CHECK ((access_account_id IS NULL) = (purpose = 5)),
    CHECK (purpose <> 1 OR masklen(network) = CASE family(network) WHEN 4 THEN 32 ELSE 128 END),
    CHECK (purpose NOT IN (2, 3) OR masklen(network) < CASE family(network) WHEN 4 THEN 32 ELSE 128 END),
    CHECK (purpose <> 3 OR family(network) = 6),
    CHECK ((link_host IS NOT NULL) = (purpose = 2 AND family(network) = 4)),
    CHECK (link_host IS NULL OR (
        family(link_host) = 4 AND masklen(link_host) = 32 AND link_host <<= network
        AND (masklen(network) = 31 OR (link_host <> host(network)::inet AND link_host <> host(broadcast(network))::inet))
    )),
    CHECK (next_hop IS NULL OR (purpose = 4 AND family(next_hop) = family(network)
        AND masklen(next_hop) = CASE family(network) WHEN 4 THEN 32 ELSE 128 END)),
    CHECK (purpose <> 4 OR family(network) = 4 OR
        (next_hop IS NOT NULL AND next_hop <> '::'::inet)),
    CHECK ((claim_key IS NULL) = (expires_at IS NULL)),
    CHECK (expires_at IS NULL OR (purpose <> 5 AND expires_at > assigned_at)),
    CHECK (released_at IS NULL OR released_at >= assigned_at),
    EXCLUDE USING gist (space_id WITH =, network inet_ops WITH &&) WHERE (released_at IS NULL)
);

CREATE INDEX rc_ip_assignments_active_account_idx
    ON rc_ip_assignments (access_account_id) WHERE released_at IS NULL;
CREATE INDEX rc_ip_assignments_active_pool_idx
    ON rc_ip_assignments (pool_id, network) WHERE released_at IS NULL;
CREATE UNIQUE INDEX rc_ip_assignments_active_claim_idx
    ON rc_ip_assignments (pool_id, claim_key) WHERE released_at IS NULL AND claim_key IS NOT NULL;
CREATE INDEX rc_ip_assignments_expiry_idx
    ON rc_ip_assignments (expires_at) WHERE released_at IS NULL AND expires_at IS NOT NULL;

-- Pool geometry is immutable. A new pool is used when its space or prefix changes.
CREATE FUNCTION rc_ip_pool_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.id, NEW.space_id, NEW.network, NEW.allocation_prefix_len)
        IS DISTINCT FROM (OLD.id, OLD.space_id, OLD.network, OLD.allocation_prefix_len) THEN
        RAISE EXCEPTION 'IP pool geometry is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER rc_ip_pool_guard
    BEFORE UPDATE ON rc_ip_pools FOR EACH ROW EXECUTE FUNCTION rc_ip_pool_guard();

CREATE FUNCTION rc_ip_assignment_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    parent rc_ip_pools%ROWTYPE;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.id, NEW.space_id, NEW.pool_id, NEW.access_account_id, NEW.network,
            NEW.purpose, NEW.next_hop, NEW.link_host, NEW.claim_key, NEW.assigned_at)
            IS DISTINCT FROM
           (OLD.id, OLD.space_id, OLD.pool_id, OLD.access_account_id, OLD.network,
            OLD.purpose, OLD.next_hop, OLD.link_host, OLD.claim_key, OLD.assigned_at)
           OR (OLD.released_at IS NOT NULL AND NEW IS DISTINCT FROM OLD) THEN
            RAISE EXCEPTION 'IP assignment identity and released records are immutable'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    SELECT * INTO parent FROM rc_ip_pools
    WHERE id = NEW.pool_id AND space_id = NEW.space_id FOR SHARE;
    IF NOT FOUND OR NOT parent.enabled OR NOT (NEW.network <<= parent.network) THEN
        RAISE EXCEPTION 'IP assignment must fit an enabled pool in the same routing space'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.claim_key IS NOT NULL AND
        (parent.allocation_prefix_len IS NULL OR
         masklen(NEW.network) <> parent.allocation_prefix_len) THEN
        RAISE EXCEPTION 'dynamic assignment must match the configured pool prefix length'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER rc_ip_assignment_guard
    BEFORE INSERT OR UPDATE ON rc_ip_assignments
    FOR EACH ROW EXECUTE FUNCTION rc_ip_assignment_guard();

-- Callers may lock the same pool row before computing a candidate from active assignments.
CREATE FUNCTION rc_ip_claim_dynamic(
    p_pool_id integer, p_account_id bigint, p_key rc_digest32, p_network cidr,
    p_purpose smallint, p_expires_at timestamptz,
    p_next_hop inet DEFAULT NULL, p_link_host inet DEFAULT NULL
) RETURNS rc_ip_assignments LANGUAGE plpgsql AS $$
DECLARE
    parent rc_ip_pools%ROWTYPE;
    assignment rc_ip_assignments%ROWTYPE;
BEGIN
    IF p_account_id IS NULL OR p_key IS NULL OR p_expires_at IS NULL OR p_purpose IS NULL THEN
        RAISE EXCEPTION 'dynamic claims require an account, claim key, purpose, and expiry'
            USING ERRCODE = '23502';
    END IF;
    SELECT * INTO STRICT parent FROM rc_ip_pools WHERE id = p_pool_id FOR UPDATE;
    SELECT * INTO assignment FROM rc_ip_assignments
    WHERE pool_id = p_pool_id AND claim_key = p_key AND released_at IS NULL;
    IF FOUND THEN
        IF (assignment.access_account_id, assignment.purpose, assignment.next_hop, assignment.link_host)
            IS DISTINCT FROM (p_account_id, p_purpose, p_next_hop, p_link_host) THEN
            RAISE EXCEPTION 'claim key belongs to a different assignment request'
                USING ERRCODE = '23505';
        END IF;
        IF assignment.expires_at <= clock_timestamp() THEN
            RAISE EXCEPTION 'expired claim needs reconciliation before reissue'
                USING ERRCODE = '23514';
        END IF;
        RETURN assignment;
    END IF;
    INSERT INTO rc_ip_assignments (
        space_id, pool_id, access_account_id, network, purpose, next_hop, link_host, claim_key, expires_at
    ) VALUES (
        parent.space_id, parent.id, p_account_id, p_network, p_purpose,
        p_next_hop, p_link_host, p_key, p_expires_at
    ) RETURNING * INTO assignment;
    RETURN assignment;
END;
$$;

CREATE FUNCTION rc_ip_renew_dynamic(
    p_id bigint, p_key rc_digest32, p_expires_at timestamptz
) RETURNS boolean LANGUAGE sql AS $$
    WITH renewed AS (
        UPDATE rc_ip_assignments SET expires_at = GREATEST(expires_at, p_expires_at)
        WHERE id = p_id AND claim_key = p_key AND released_at IS NULL
            AND p_expires_at > clock_timestamp() AND expires_at > clock_timestamp()
        RETURNING id
    ) SELECT EXISTS (SELECT 1 FROM renewed);
$$;

-- Expiry-based release requires the previously examined expiry as a compare-and-swap guard.
-- The application calls this after verifying that the NAS/session no longer uses the resource.
CREATE FUNCTION rc_ip_release_expired(
    p_id bigint, p_key rc_digest32, p_expected_expiry timestamptz
) RETURNS boolean LANGUAGE sql AS $$
    WITH released AS (
        UPDATE rc_ip_assignments SET released_at = clock_timestamp()
        WHERE id = p_id AND claim_key = p_key AND released_at IS NULL
            AND expires_at = p_expected_expiry AND expires_at <= clock_timestamp()
        RETURNING id
    ) SELECT EXISTS (SELECT 1 FROM released);
$$;

INSERT INTO rc_attribute_defs (wire_path, name, value_type, min_octets, max_octets)
VALUES
    (ARRAY[9], 'Framed-IP-Netmask', 'ipv4addr', 4, 4),
    (ARRAY[22], 'Framed-Route', 'text', 1, 253),
    (ARRAY[88], 'Framed-Pool', 'text', 1, 253),
    (ARRAY[99], 'Framed-IPv6-Route', 'text', 1, 253),
    (ARRAY[100], 'Framed-IPv6-Pool', 'text', 1, 253),
    (ARRAY[168], 'Framed-IPv6-Address', 'ipv6addr', 16, 16),
    (ARRAY[171], 'Delegated-IPv6-Prefix-Pool', 'text', 1, 253);
