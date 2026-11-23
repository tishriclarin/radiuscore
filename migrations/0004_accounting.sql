-- Session keys and the NAS epoch must be resolved from verified packet context.
-- This function persists resolved Start, Stop, and Interim-Update events atomically.
CREATE FUNCTION rc_acct_record_session_event(
    p_nas_id integer,
    p_nas_epoch bigint,
    p_session_key rc_digest32,
    p_acct_session_id bytea,
    p_account_id bigint,
    p_event_key rc_digest32,
    p_status rc_uint32,
    p_event_at timestamptz,
    p_input_octets rc_uint64 DEFAULT NULL,
    p_output_octets rc_uint64 DEFAULT NULL,
    p_input_packets rc_uint32 DEFAULT NULL,
    p_output_packets rc_uint32 DEFAULT NULL,
    p_session_seconds rc_uint32 DEFAULT NULL,
    p_terminate_cause rc_uint32 DEFAULT NULL
) RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE
    session_row rc_acct_sessions%ROWTYPE;
    event_id bigint;
BEGIN
    IF p_status IS NULL OR p_status NOT IN (1, 2, 3) OR p_event_at IS NULL THEN
        RAISE EXCEPTION 'a resolved session event requires Start, Stop or Interim and an effective timestamp'
            USING ERRCODE = '23514';
    END IF;
    INSERT INTO rc_acct_events (
        nas_id, event_key, status_type, event_at, input_octets, output_octets,
        input_packets, output_packets, session_seconds, terminate_cause
    ) VALUES (
        p_nas_id, p_event_key, p_status, p_event_at, p_input_octets, p_output_octets,
        p_input_packets, p_output_packets, p_session_seconds, p_terminate_cause
    ) ON CONFLICT (nas_id, event_key) DO NOTHING
    RETURNING id INTO event_id;
    IF NOT FOUND THEN
        SELECT id INTO STRICT event_id FROM rc_acct_events
        WHERE nas_id = p_nas_id AND event_key = p_event_key;
        RETURN event_id;
    END IF;

    INSERT INTO rc_acct_sessions (
        nas_id, nas_epoch, session_key, acct_session_id, access_account_id, last_seen_at
    ) VALUES (
        p_nas_id, p_nas_epoch, p_session_key, p_acct_session_id, p_account_id, p_event_at
    ) ON CONFLICT (nas_id, nas_epoch, session_key) DO NOTHING;

    SELECT * INTO STRICT session_row FROM rc_acct_sessions
    WHERE nas_id = p_nas_id AND nas_epoch = p_nas_epoch AND session_key = p_session_key
    FOR UPDATE;
    IF session_row.acct_session_id <> p_acct_session_id OR
        (session_row.access_account_id IS NOT NULL AND p_account_id IS NOT NULL
            AND session_row.access_account_id <> p_account_id) THEN
        RAISE EXCEPTION 'accounting event does not match the bound session identity'
            USING ERRCODE = '23514';
    END IF;

    UPDATE rc_acct_sessions SET
        access_account_id = COALESCE(access_account_id, p_account_id),
        started_at = CASE
            WHEN p_status = 1 AND (ended_at IS NULL OR p_event_at <= ended_at)
            THEN LEAST(started_at, p_event_at)
            ELSE started_at
        END,
        last_seen_at = CASE
            WHEN ended_at IS NULL THEN GREATEST(last_seen_at, p_event_at)
            ELSE last_seen_at
        END,
        ended_at = CASE
            WHEN p_status = 2 AND ended_at IS NULL AND (started_at IS NULL OR p_event_at >= started_at)
            THEN p_event_at
            ELSE ended_at
        END,
        input_octets = CASE
            WHEN ended_at IS NULL AND (p_status = 2 OR p_event_at >= last_seen_at)
                AND (started_at IS NULL OR p_event_at >= started_at)
            THEN GREATEST(input_octets, p_input_octets) ELSE input_octets END,
        output_octets = CASE
            WHEN ended_at IS NULL AND (p_status = 2 OR p_event_at >= last_seen_at)
                AND (started_at IS NULL OR p_event_at >= started_at)
            THEN GREATEST(output_octets, p_output_octets) ELSE output_octets END,
        input_packets = CASE
            WHEN ended_at IS NULL AND (p_status = 2 OR p_event_at >= last_seen_at)
                AND (started_at IS NULL OR p_event_at >= started_at)
            THEN GREATEST(input_packets, p_input_packets) ELSE input_packets END,
        output_packets = CASE
            WHEN ended_at IS NULL AND (p_status = 2 OR p_event_at >= last_seen_at)
                AND (started_at IS NULL OR p_event_at >= started_at)
            THEN GREATEST(output_packets, p_output_packets) ELSE output_packets END,
        session_seconds = CASE
            WHEN ended_at IS NULL AND (p_status = 2 OR p_event_at >= last_seen_at)
                AND (started_at IS NULL OR p_event_at >= started_at)
            THEN GREATEST(session_seconds, p_session_seconds) ELSE session_seconds END,
        terminate_cause = CASE
            WHEN p_status = 2 AND ended_at IS NULL AND (started_at IS NULL OR p_event_at >= started_at)
            THEN p_terminate_cause ELSE terminate_cause END
    WHERE id = session_row.id;

    UPDATE rc_acct_events SET session_id = session_row.id WHERE id = event_id;
    RETURN event_id;
END;
$$;
