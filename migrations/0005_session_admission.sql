-- This is an internal policy, never a RADIUS reply attribute.
ALTER TABLE rc_access_accounts ADD COLUMN simultaneous_users integer
    CHECK (simultaneous_users > 0);

CREATE TABLE rc_session_admissions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    access_account_id bigint NOT NULL REFERENCES rc_access_accounts,
    context_key rc_digest32 NOT NULL,
    granted_at timestamptz NOT NULL,
    lease_expires_at timestamptz,
    ended_at timestamptz,
    CHECK (lease_expires_at IS NULL OR lease_expires_at > granted_at),
    CHECK (ended_at IS NULL OR ended_at >= granted_at)
);

CREATE UNIQUE INDEX rc_session_admissions_open_context_idx
    ON rc_session_admissions (access_account_id, context_key) WHERE ended_at IS NULL;

-- Invoke only for a verified and authorized access context. NULL means unlimited.
CREATE FUNCTION rc_admit_session(
    p_account_id bigint, p_context_key rc_digest32, p_lease_expires_at timestamptz DEFAULT NULL
) RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE
    allowed_count integer;
    enabled_account boolean;
    current_id bigint;
    current_expiry timestamptz;
    observed_at timestamptz;
BEGIN
    -- After waiting on the account lock the count must see the winner's commit.
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'session admission requires READ COMMITTED isolation'
            USING ERRCODE = '0A000';
    END IF;
    SELECT a.simultaneous_users, a.enabled INTO allowed_count, enabled_account
      FROM rc_access_accounts a WHERE a.id = p_account_id FOR UPDATE;
    IF NOT FOUND OR NOT enabled_account OR p_context_key IS NULL THEN
        RETURN NULL;
    END IF;
    observed_at := clock_timestamp();
    IF p_lease_expires_at IS NOT NULL AND p_lease_expires_at <= observed_at THEN
        RETURN NULL;
    END IF;
    SELECT id, lease_expires_at INTO current_id, current_expiry FROM rc_session_admissions
    WHERE access_account_id = p_account_id AND context_key = p_context_key AND ended_at IS NULL;
    IF FOUND THEN
        IF (current_expiry IS NULL) <> (p_lease_expires_at IS NULL)
           OR current_expiry <= observed_at THEN
            RETURN NULL;
        END IF;
        RETURN current_id;
    END IF;
    IF allowed_count IS NOT NULL AND (
        SELECT count(*) FROM rc_session_admissions
        WHERE access_account_id = p_account_id AND ended_at IS NULL
    ) >= allowed_count THEN
        RETURN NULL;
    END IF;
    INSERT INTO rc_session_admissions (access_account_id, context_key, granted_at, lease_expires_at)
    VALUES (p_account_id, p_context_key, observed_at, p_lease_expires_at) RETURNING id INTO current_id;
    RETURN current_id;
END;
$$;

-- Refresh only after the authoritative lease source confirms renewal. These operations
-- share the account lock and compare the prior expiry to serialize renewal and teardown.
CREATE FUNCTION rc_renew_session_admission(
    p_id bigint, p_previous_expiry timestamptz, p_new_expiry timestamptz
) RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM rc_access_accounts
    WHERE id = (SELECT access_account_id FROM rc_session_admissions WHERE id = p_id)
      AND enabled FOR UPDATE;
    IF NOT FOUND THEN RETURN false; END IF;
    UPDATE rc_session_admissions SET lease_expires_at = p_new_expiry
    WHERE id = p_id AND ended_at IS NULL AND lease_expires_at = p_previous_expiry
      AND lease_expires_at > clock_timestamp() AND p_new_expiry > lease_expires_at;
    RETURN FOUND;
END;
$$;

-- Invoke only after authoritative expiry or verified disappearance. Match the
-- admission generation and current expiry so stale events cannot close a renewal.
CREATE FUNCTION rc_close_session_admission(p_id bigint, p_previous_expiry timestamptz)
RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM rc_access_accounts
    WHERE id = (SELECT access_account_id FROM rc_session_admissions WHERE id = p_id) FOR UPDATE;
    IF NOT FOUND THEN RETURN false; END IF;
    UPDATE rc_session_admissions SET ended_at = clock_timestamp()
    WHERE id = p_id AND ended_at IS NULL
      AND lease_expires_at IS NOT DISTINCT FROM p_previous_expiry;
    RETURN FOUND;
END;
$$;

-- Reconcile an existing accounting row after the authoritative device confirms
-- the session is gone. A concurrent update invalidates the observation.
CREATE FUNCTION rc_reconcile_accounting_session(p_id bigint, p_observed_last_seen timestamptz)
RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    UPDATE rc_acct_sessions
       SET ended_at = GREATEST(clock_timestamp(), last_seen_at, started_at)
     WHERE id = p_id AND ended_at IS NULL AND last_seen_at = p_observed_last_seen;
    RETURN FOUND;
END;
$$;
