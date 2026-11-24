#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
state=$(mktemp -d)
trap 'rm -rf "$state"' EXIT
export RADIUSCORE_SIM_STATE="$state"
run="$root/simulation/run.sh"

psql -Xqv ON_ERROR_STOP=1 -c 'CREATE TABLE history_marker (value integer); INSERT INTO history_marker VALUES (27)'
"$run" --database --seed 27 > "$state/first.json"
first=$(jq -er '.test_id | select(test("^[0-9a-f]{16}$"))' "$state/first.json")

psql -Xqv ON_ERROR_STOP=1 -c 'UPDATE history_marker SET value=99; CREATE TABLE after_snapshot (value integer)'
"$run" --extend "$first" --rerun "$first" --rerun-opt db-config --seed 109 > "$state/replayed.json"
jq -e --arg id "$first" '.status=="passed" and .selection=="extend" and .extends==$id and .rerun_of==$id and .seed==109 and (.notices|any(contains("Default selection is extend")))' "$state/replayed.json"
[ "$(psql -XqAtv ON_ERROR_STOP=1 -c "SELECT value FROM history_marker")" = 27 ]
[ -z "$(psql -XqAtv ON_ERROR_STOP=1 -c "SELECT to_regclass('public.after_snapshot')")" ]

"$run" --database --clear > "$state/cleared.json"
backup=$(jq -er '.snapshots.before_change | select(type=="string")' "$state/cleared.json")
[ -s "$state/$backup" ]
[ -z "$(psql -XqAtv ON_ERROR_STOP=1 -c "SELECT to_regclass('public.history_marker')")" ]
"$run" --rerun "$first" --rerun-opt none --seed 3 > "$state/none.json"
jq -e '.status=="passed" and .seed==3 and .snapshots.before_change==null' "$state/none.json"
[ -z "$(psql -XqAtv ON_ERROR_STOP=1 -c "SELECT to_regclass('public.history_marker')")" ]

"$run" --rerun "$first" --rerun-opt config > "$state/config.json"
jq -e '.status=="passed" and .seed==27 and .snapshots.before_change==null' "$state/config.json"
"$run" --journal | jq -e '.status=="passed" and (.runs|length)==5'
