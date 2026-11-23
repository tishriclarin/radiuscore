BEGIN;
DO $$
DECLARE
    test_site integer;
    test_nas integer;
    profile integer;
    account bigint;
    first_event bigint;
    retried_event bigint;
    session_row rc_acct_sessions%ROWTYPE;
    session_key bytea := decode(repeat('ff', 32), 'hex');
    wire_session bytea := convert_to('accounting-lab-session', 'UTF8');
    origin timestamptz := '2026-01-01 00:00:00+00';
BEGIN
    INSERT INTO rc_sites (code) VALUES ('test-accounting') RETURNING id INTO test_site;
    INSERT INTO rc_nas (site_id, code) VALUES (test_site, 'test-accounting') RETURNING id INTO test_nas;
    INSERT INTO rc_service_profiles (code) VALUES ('test-accounting') RETURNING id INTO profile;
    INSERT INTO rc_access_accounts (service_profile_id) VALUES (profile) RETURNING id INTO account;
    first_event := rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('01',32),'hex'), 1, origin);
    retried_event := rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('01',32),'hex'), 1, origin);
    IF first_event <> retried_event THEN
        RAISE EXCEPTION 'accounting retransmission returned a different event ID';
    END IF;
    PERFORM rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('02',32),'hex'), 3, origin + interval '4 minutes', 18446744073709551614);
    PERFORM rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('03',32),'hex'), 3, origin + interval '3 minutes', 10);
    PERFORM rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('04',32),'hex'), 2, origin + interval '5 minutes', 18446744073709551615);
    PERFORM rc_acct_record_session_event(test_nas, 1, session_key, wire_session, account,
        decode(repeat('05',32),'hex'), 3, origin + interval '6 minutes', 0);
    SELECT * INTO STRICT session_row FROM rc_acct_sessions WHERE nas_id = test_nas;
    IF session_row.started_at IS DISTINCT FROM origin OR
        session_row.ended_at IS DISTINCT FROM origin + interval '5 minutes' OR
        session_row.input_octets IS DISTINCT FROM 18446744073709551615 OR
        (SELECT count(*) FROM rc_acct_events WHERE nas_id = test_nas) <> 5 OR
        (SELECT count(*) FROM rc_acct_events WHERE session_id = session_row.id) <> 5 THEN
        RAISE EXCEPTION 'session accounting projection is inconsistent';
    END IF;
    BEGIN
        PERFORM rc_acct_record_session_event(test_nas, 1, session_key, convert_to('wrong', 'UTF8'),
            account, decode(repeat('06',32),'hex'), 3, origin);
        RAISE EXCEPTION 'a different wire session ID attached to the same session';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
    IF (SELECT count(*) FROM rc_acct_events WHERE nas_id = test_nas) <> 5 THEN
        RAISE EXCEPTION 'failed session update left an accounting event behind';
    END IF;
END;
$$;
ROLLBACK;
