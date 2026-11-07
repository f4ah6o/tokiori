#!/usr/bin/env bash
# Registers the golden-scenario organization with a running Tokiori database.
#
#   DATABASE_URL=postgres://tokiori:tokiori@localhost:55432/tokiori ./seed.sh
#
# Set TOKIORI_SCOPE to write into a scope other than `default`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
tokiori="${TOKIORI:-cargo run -q -p tokiori-cli --}"
scope="${TOKIORI_SCOPE:-default}"

$tokiori --scope "$scope" model apply --file "$here/model.json"
$tokiori --scope "$scope" entity create --type employee --id E001 \
  --attributes '{"name":"Alice","joined_on":"2027-04-01"}'
$tokiori --scope "$scope" entity create --type department --id D-SALES --attributes '{"name":"Sales"}'
$tokiori --scope "$scope" entity create --type department --id D-PLANNING --attributes '{"name":"Planning"}'
$tokiori --scope "$scope" relation create --type membership --id M001 \
  --role employee=E001 --role department=D-SALES \
  --attributes '{"title":"Account manager"}' --valid-from 2027-04-01
