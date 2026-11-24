-- Transaction rolls back all fixtures. Run only against a disposable database.
BEGIN;
CREATE TEMP TABLE sim_results (position integer GENERATED ALWAYS AS IDENTITY, name text, expected jsonb, actual jsonb);
SELECT setseed(mod(:'seed'::numeric, 1000000)::double precision / 1000000);

DO $$
DECLARE
    site integer;
    nas integer;
    profile integer;
    account bigint;
    pool integer;
    space integer;
    item integer;
    key_a rc_digest32 := decode(repeat('01', 32), 'hex');
    key_b rc_digest32 := decode(repeat('02', 32), 'hex');
    e1 rc_digest32 := decode(repeat('11', 32), 'hex');
    e2 rc_digest32 := decode(repeat('12', 32), 'hex');
    e3 rc_digest32 := decode(repeat('13', 32), 'hex');
    t timestamptz := clock_timestamp();
    expiry timestamptz := t + interval '10 minutes';
    renewal timestamptz := t + interval '15 minutes';
    slot bigint;
    lease rc_ip_assignments;
    second_lease rc_ip_assignments;
    expired_ip rc_ip_assignments;
    ip_reused boolean;
    session_row rc_acct_sessions;
    refreshed boolean;
    renewed_ip boolean;
    prior_stop_ignored boolean;
    second_client_blocked boolean;
    cleaned boolean;
BEGIN
    INSERT INTO rc_sites(code) VALUES ('simulation') RETURNING id INTO site;
    INSERT INTO rc_nas(site_id, code) VALUES (site, 'simulation') RETURNING id INTO nas;
    INSERT INTO rc_service_profiles(code) VALUES ('simulation') RETURNING id INTO profile;
    INSERT INTO rc_ip_spaces(code) VALUES ('simulation') RETURNING id INTO space;
    INSERT INTO rc_ip_pools(space_id, code, network, allocation_prefix_len)
        VALUES (space, 'simulation', '192.0.2.0/24', 32) RETURNING id INTO pool;

    -- Run accounts in seeded random order while keeping each account's stages ordered.
    FOR item IN SELECT value FROM generate_series(10,12) AS g(value) ORDER BY random() LOOP
        INSERT INTO rc_access_accounts(service_profile_id, simultaneous_users)
            VALUES (profile, 1) RETURNING id INTO account;
        slot := rc_admit_session(account, key_a, NULL);
        lease := rc_ip_claim_dynamic(pool, account, sha256(convert_to('lease' || item, 'UTF8')),
            ('192.0.2.' || item || '/32')::cidr, 1::smallint, expiry);
        PERFORM rc_acct_record_session_event(nas, 1, sha256(convert_to('session' || item, 'UTF8')), decode('01','hex'), account,
            sha256(convert_to('start' || item, 'UTF8')), 1, t);
        PERFORM rc_acct_record_session_event(nas, 1, sha256(convert_to('session' || item, 'UTF8')), decode('01','hex'), account,
            sha256(convert_to('interim' || item, 'UTF8')), 3, t + interval '1 minute', 100);
        PERFORM rc_acct_record_session_event(nas, 1, sha256(convert_to('session' || item, 'UTF8')), decode('01','hex'), account,
            sha256(convert_to('stop' || item, 'UTF8')), 2, t + interval '2 minutes', 200);
        cleaned := rc_close_session_admission(slot, NULL);
        INSERT INTO sim_results(name,expected,actual) VALUES (
            'ordered_session_' || item, '{"admitted":true,"allocated":true,"closed":true,"input_octets":200}',
            jsonb_build_object('admitted',slot IS NOT NULL,'allocated',lease.id IS NOT NULL,
                'closed',cleaned AND (SELECT ended_at IS NOT NULL FROM rc_acct_sessions WHERE access_account_id=account),
                'input_octets',(SELECT input_octets FROM rc_acct_sessions WHERE access_account_id=account)));
    END LOOP;

    INSERT INTO rc_access_accounts(service_profile_id, simultaneous_users)
        VALUES (profile, 1) RETURNING id INTO account;
    PERFORM rc_acct_record_session_event(nas, 1, key_b, decode('02','hex'), account, e2, 2, t + interval '2 minutes', 200);
    -- Retransmit the stop, then randomize the order of the delayed start and interim.
    PERFORM rc_acct_record_session_event(nas, 1, key_b, decode('02','hex'), account, e2, 2, t + interval '2 minutes', 200);
    FOR item IN SELECT value FROM generate_series(1,3,2) AS g(value) ORDER BY random() LOOP
        IF item = 1 THEN
            PERFORM rc_acct_record_session_event(nas, 1, key_b, decode('02','hex'), account, e1, 1, t);
        ELSE
            PERFORM rc_acct_record_session_event(nas, 1, key_b, decode('02','hex'), account, e3, 3, t + interval '1 minute', 100);
        END IF;
    END LOOP;
    INSERT INTO sim_results(name,expected,actual)
    SELECT 'out_of_order_and_duplicate_accounting', '{"closed":true,"events":3,"input_octets":200}',
           jsonb_build_object('closed',s.ended_at IS NOT NULL,'events',count(e.id),'input_octets',s.input_octets)
      FROM rc_acct_sessions s LEFT JOIN rc_acct_events e ON s.id=e.session_id
     WHERE s.nas_id=nas AND s.session_key=key_b GROUP BY s.id;

    INSERT INTO rc_access_accounts(service_profile_id, simultaneous_users)
        VALUES (profile, 1) RETURNING id INTO account;
    slot := rc_admit_session(account, key_a, expiry);
    lease := rc_ip_claim_dynamic(pool, account, key_a, '192.0.2.20', 1::smallint, expiry);
    second_lease := rc_ip_claim_dynamic(pool, account, key_a, '192.0.2.21', 1::smallint, expiry);
    refreshed := rc_renew_session_admission(slot, expiry, renewal);
    renewed_ip := rc_ip_renew_dynamic(lease.id, key_a, renewal);
    prior_stop_ignored := NOT rc_close_session_admission(slot, expiry);
    second_client_blocked := rc_admit_session(account, key_b, renewal) IS NULL;
    INSERT INTO sim_results(name,expected,actual) VALUES (
        'dhcp_t1_single_slot_renewal',
        '{"same_slot":true,"same_ip":true,"renewed":true,"stale_stop_ignored":true,"other_client_blocked":true}',
        jsonb_build_object('same_slot',slot IS NOT NULL AND rc_admit_session(account,key_a,renewal)=slot,
          'same_ip',second_lease.network=lease.network,
          'renewed',refreshed AND renewed_ip,'stale_stop_ignored',prior_stop_ignored,'other_client_blocked',second_client_blocked));

    PERFORM rc_acct_record_session_event(nas, 1, key_a, decode('03','hex'), account, decode(repeat('33',32),'hex'), 1, t);
    SELECT * INTO STRICT session_row FROM rc_acct_sessions WHERE nas_id=nas AND session_key=key_a;
    INSERT INTO sim_results(name,expected,actual) VALUES (
        'abrupt_missing_stop_detected', '{"open_accounting":1,"open_admissions":1}',
        jsonb_build_object('open_accounting',(SELECT count(*) FROM rc_acct_sessions WHERE access_account_id=account AND ended_at IS NULL),
                           'open_admissions',(SELECT count(*) FROM rc_session_admissions WHERE access_account_id=account AND ended_at IS NULL)));
    -- Lab fixture supplies an authoritative report that the old session disappeared.
    cleaned := rc_reconcile_accounting_session(session_row.id, session_row.last_seen_at);
    cleaned := rc_close_session_admission(slot, renewal) AND cleaned;
    INSERT INTO sim_results(name,expected,actual) VALUES (
        'confirmed_disconnect_reconciles_leftovers', '{"closed":true,"open_accounting":0,"open_admissions":0}',
        jsonb_build_object('closed',cleaned,
          'open_accounting',(SELECT count(*) FROM rc_acct_sessions WHERE access_account_id=account AND ended_at IS NULL),
          'open_admissions',(SELECT count(*) FROM rc_session_admissions WHERE access_account_id=account AND ended_at IS NULL)));

    -- Seed a historical lease that passed expiry. It continues to occupy its slot
    -- until the lab models authoritative expiry reconciliation.
    INSERT INTO rc_session_admissions(access_account_id,context_key,granted_at,lease_expires_at)
        VALUES(account,key_a,t-interval '11 minutes',t-interval '1 minute') RETURNING id INTO slot;
    second_client_blocked := rc_admit_session(account,key_b,renewal) IS NULL;
    cleaned := rc_close_session_admission(slot,t-interval '1 minute');
    INSERT INTO rc_ip_assignments(space_id,pool_id,access_account_id,network,purpose,claim_key,assigned_at,expires_at)
        VALUES(space,pool,account,'192.0.2.25',1,key_b,t-interval '11 minutes',t-interval '1 minute')
        RETURNING * INTO expired_ip;
    ip_reused := rc_ip_release_expired(expired_ip.id,key_b,expired_ip.expires_at);
    second_lease := rc_ip_claim_dynamic(pool,account,key_b,expired_ip.network,1::smallint,renewal);
    INSERT INTO sim_results(name,expected,actual) VALUES (
        'expired_lease_reconciles_before_reuse',
        '{"held_until_reconciled":true,"closed":true,"new_context_admitted":true,"ip_reused":true}',
        jsonb_build_object('held_until_reconciled',second_client_blocked,'closed',cleaned,
            'new_context_admitted',rc_admit_session(account,key_b,renewal) IS NOT NULL,
            'ip_reused',ip_reused AND second_lease.id<>expired_ip.id AND second_lease.network=expired_ip.network));

    UPDATE rc_access_accounts SET enabled=false WHERE id=account;
    INSERT INTO sim_results(name,expected,actual) VALUES (
        'deactivated_account_denies_access', '{"existing_blocked":true,"new_blocked":true}',
        jsonb_build_object('existing_blocked',rc_admit_session(account,key_b,renewal) IS NULL,
                           'new_blocked',rc_admit_session(account,key_a,renewal) IS NULL));
END;
$$;

SELECT json_build_object('name',name,'expected',expected,'actual',actual)
FROM sim_results ORDER BY position;
ROLLBACK;
