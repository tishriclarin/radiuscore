#!/bin/sh
set -eu
: "${PGDATABASE:?Set PGDATABASE to a fresh, disposable test database}"
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
psql -X --single-transaction -v ON_ERROR_STOP=1 \
    -f "$project_dir/migrations/0001_schema.sql" \
    -f "$project_dir/migrations/0002_dictionary.sql" \
    -f "$project_dir/migrations/0003_ipam.sql" \
    -f "$project_dir/migrations/0004_accounting.sql" \
    -f "$project_dir/migrations/0005_session_admission.sql"
psql -X -v ON_ERROR_STOP=1 -f "$project_dir/tests/ipam.sql"
psql -X -v ON_ERROR_STOP=1 -f "$project_dir/tests/accounting.sql"
python3 "$project_dir/tests/ipam_concurrency.py"
"$project_dir/simulation/run.sh" --database --seed 27
"$project_dir/simulation/history-test.sh"
