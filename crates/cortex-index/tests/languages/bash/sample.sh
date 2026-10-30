#!/usr/bin/env bash
# Deploy: a small script that uses each construct the Bash queries read.
source ./lib/common.sh
. "./lib/log.sh"

readonly RATE=2
declare -r LIMIT=3
typeset -r MODE=fast

function deploy() {
  local target="$1"
  build "$target" | tee deploy.log
  notify "done"
}

function rollback {
  restore "$1"
}

cleanup() {
  rm -f "$TMP"
}

deploy prod
