"""Compete for one address with separate PostgreSQL connections."""
import os
import subprocess
import time

PSQL = ["psql", "-X", "-Atq", "-v", "ON_ERROR_STOP=1"]


def query(statement):
    return subprocess.check_output([*PSQL, "-c", statement], text=True, timeout=10).strip()


def main():
    query("""
        INSERT INTO rc_ip_spaces (code) VALUES ('test-concurrency');
        INSERT INTO rc_ip_pools (space_id, code, network, allocation_prefix_len)
        SELECT id, 'test-concurrency', '198.51.100.0/30', 32 FROM rc_ip_spaces
        WHERE code = 'test-concurrency';
        INSERT INTO rc_service_profiles (code) VALUES ('test-concurrency');
        INSERT INTO rc_access_accounts (service_profile_id)
        SELECT id FROM rc_service_profiles WHERE code = 'test-concurrency';
    """)
    first = None
    try:
        pool = int(query("SELECT id FROM rc_ip_pools WHERE code = 'test-concurrency'"))
        account = int(query("""
            SELECT a.id FROM rc_access_accounts a
            JOIN rc_service_profiles p ON a.service_profile_id = p.id
            WHERE p.code = 'test-concurrency'
        """))
        base = f"({pool}, {account}, {{key}}, '198.51.100.1/32', 1::smallint, now() + interval '1 hour')"
        winner = base.format(key="decode(repeat('31',32),'hex')")
        loser = base.format(key="decode(repeat('32',32),'hex')")
        first = subprocess.Popen(
            [*PSQL, "-c", f"BEGIN; SELECT rc_ip_claim_dynamic{winner}; SELECT pg_sleep(2); COMMIT;"],
            env={**os.environ, "PGAPPNAME": "rc_ip_claim_concurrency"},
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            waiting = query("""
                SELECT count(*) FROM pg_stat_activity
                WHERE application_name = 'rc_ip_claim_concurrency' AND wait_event = 'PgSleep'
            """)
            if waiting == "1":
                break
            if first.poll() is not None:
                raise AssertionError(f"winning claim did not reach its hold point: {first.communicate()}")
            time.sleep(0.05)
        else:
            raise AssertionError("timed out waiting for the first claim")

        rejected = subprocess.run(
            [*PSQL, "-c", f"DO $$ BEGIN PERFORM rc_ip_claim_dynamic{loser}; "
             "RAISE EXCEPTION 'overlapping claim succeeded'; "
             "EXCEPTION WHEN exclusion_violation THEN NULL; END $$;"],
            text=True, capture_output=True, timeout=10,
        )
        if rejected.returncode:
            raise AssertionError(rejected.stderr)
        _, error = first.communicate(timeout=10)
        if first.returncode:
            raise AssertionError(error)
        assert query(f"SELECT count(*) FROM rc_ip_assignments WHERE pool_id = {pool}") == "1"
        print("Concurrent claim test passed: one assignment, overlapping writer rejected")
    finally:
        if first is not None and first.poll() is None:
            first.kill()
            first.communicate()
        query("""
            DELETE FROM rc_ip_assignments WHERE pool_id IN (
                SELECT id FROM rc_ip_pools WHERE code = 'test-concurrency');
            DELETE FROM rc_ip_pools WHERE code = 'test-concurrency';
            DELETE FROM rc_ip_spaces WHERE code = 'test-concurrency';
            DELETE FROM rc_access_accounts WHERE service_profile_id IN (
                SELECT id FROM rc_service_profiles WHERE code = 'test-concurrency');
            DELETE FROM rc_service_profiles WHERE code = 'test-concurrency';
        """)


if __name__ == "__main__":
    main()
