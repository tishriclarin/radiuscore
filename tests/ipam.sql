BEGIN;
DO $$
DECLARE
    public_space integer;
    lab_space integer;
    primary_pool integer;
    lab_pool integer;
    ipv6_pool integer;
    profile integer;
    account bigint;
    delegated bigint;
    lease_key bytea := decode(repeat('02', 32), 'hex');
    original_lease rc_ip_assignments%ROWTYPE;
    retried_lease rc_ip_assignments%ROWTYPE;
    quota_key bytea := decode(repeat('01', 32), 'hex');
BEGIN
    INSERT INTO rc_ip_spaces (code) VALUES ('test-public') RETURNING id INTO public_space;
    INSERT INTO rc_ip_spaces (code) VALUES ('test-lab-vrf') RETURNING id INTO lab_space;
    INSERT INTO rc_ip_pools (space_id, code, network, allocation_prefix_len)
        VALUES (public_space, 'test-v4', '192.0.2.0/24', 32) RETURNING id INTO primary_pool;
    INSERT INTO rc_ip_pools (space_id, code, network, allocation_prefix_len)
        VALUES (lab_space, 'test-v4', '192.0.2.0/24', 32) RETURNING id INTO lab_pool;
    INSERT INTO rc_ip_pools (space_id, code, network, allocation_prefix_len)
        VALUES (public_space, 'test-v6', '2001:db8::/32', 56) RETURNING id INTO ipv6_pool;
    INSERT INTO rc_service_profiles (code) VALUES ('test-ipam') RETURNING id INTO profile;
    INSERT INTO rc_access_accounts (service_profile_id) VALUES (profile) RETURNING id INTO account;

    INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose)
        VALUES (public_space, primary_pool, account, '192.0.2.2/32', 1),
               (public_space, primary_pool, account, '192.0.2.32/29', 4),
               (lab_space, lab_pool, account, '192.0.2.32/29', 4);
    INSERT INTO rc_ip_assignments (space_id, pool_id, network, purpose)
        VALUES (public_space, primary_pool, '192.0.2.0/31', 5);
    INSERT INTO rc_ip_assignments (
        space_id, pool_id, access_account_id, network, purpose, claim_key, expires_at
    ) VALUES (public_space, ipv6_pool, account, '2001:db8:100::/56', 3,
        quota_key, clock_timestamp() + interval '1 hour') RETURNING id INTO delegated;

    BEGIN
        INSERT INTO rc_ip_pools (space_id, code, network)
            VALUES (public_space, 'nested', '192.0.2.128/25');
        RAISE EXCEPTION 'overlapping pool was accepted';
    EXCEPTION WHEN exclusion_violation THEN NULL;
    END;

    BEGIN
        INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose)
            VALUES (public_space, primary_pool, account, '192.0.2.33/32', 1);
        RAISE EXCEPTION 'overlap with routed block was accepted';
    EXCEPTION WHEN exclusion_violation THEN NULL;
    END;

    BEGIN
        INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose)
            VALUES (public_space, primary_pool, account, '198.51.100.10/32', 1);
        RAISE EXCEPTION 'out-of-pool address was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    BEGIN
        INSERT INTO rc_ip_assignments (
            space_id, pool_id, access_account_id, network, purpose, claim_key, expires_at
        ) VALUES (public_space, ipv6_pool, account, '2001:db8:200::/60', 3,
            quota_key, clock_timestamp() + interval '1 hour');
        RAISE EXCEPTION 'dynamic assignment with different prefix length was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    BEGIN
        INSERT INTO rc_ip_assignments (
            space_id, pool_id, access_account_id, network, purpose, claim_key, expires_at
        ) VALUES (public_space, ipv6_pool, account, '2001:db8:200::/56', 3,
            quota_key, clock_timestamp() + interval '1 hour');
        RAISE EXCEPTION 'duplicate active lease key was accepted';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    UPDATE rc_ip_assignments SET released_at = clock_timestamp() WHERE id = delegated;
    INSERT INTO rc_ip_assignments (
        space_id, pool_id, access_account_id, network, purpose, claim_key, expires_at
    ) VALUES (public_space, ipv6_pool, account, '2001:db8:100::/56', 3,
        quota_key, clock_timestamp() + interval '1 hour');

    BEGIN
        UPDATE rc_ip_assignments SET released_at = NULL WHERE id = delegated;
        RAISE EXCEPTION 'a released record was reactivated';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose, link_host)
        VALUES (public_space, primary_pool, account, '192.0.2.64/29', 2, '192.0.2.65');
    BEGIN
        INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose, link_host)
            VALUES (public_space, primary_pool, account, '192.0.2.72/29', 2, '192.0.2.79');
        RAISE EXCEPTION 'broadcast accepted as the IPv4 link host';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    original_lease := rc_ip_claim_dynamic(primary_pool, account, lease_key, '192.0.2.81/32', 1::smallint,
        clock_timestamp() + interval '1 hour');
    retried_lease := rc_ip_claim_dynamic(primary_pool, account, lease_key, '192.0.2.82/32', 1::smallint,
        clock_timestamp() + interval '2 hours');
    IF original_lease.id <> retried_lease.id OR original_lease.network <> retried_lease.network THEN
        RAISE EXCEPTION 'retransmission issued a different address';
    END IF;
    IF NOT rc_ip_renew_dynamic(original_lease.id, lease_key, clock_timestamp() + interval '3 hours') THEN
        RAISE EXCEPTION 'active lease failed to renew';
    END IF;
    IF rc_ip_release_expired(original_lease.id, lease_key, original_lease.expires_at) THEN
        RAISE EXCEPTION 'stale expiry released a renewed lease';
    END IF;

    UPDATE rc_ip_pools SET enabled = false WHERE id = primary_pool;
    BEGIN
        INSERT INTO rc_ip_assignments (space_id, pool_id, access_account_id, network, purpose)
            VALUES (public_space, primary_pool, account, '192.0.2.50/32', 1);
        RAISE EXCEPTION 'disabled pool granted a new assignment';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
END;
$$;
ROLLBACK;
