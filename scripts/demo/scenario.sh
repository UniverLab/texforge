#!/usr/bin/env bash
# Demo scenario for texforge — replayed inside asciinema by record.sh.
# Simulates human typing, then runs the real command.

set -e

PROMPT='\033[1;32m❯\033[0m '

type_cmd() {
    printf "$PROMPT"
    for ((i = 0; i < ${#1}; i++)); do
        printf '%s' "${1:i:1}"
        sleep 0.04
    done
    sleep 0.4
    printf '\n'
}

run() {
    type_cmd "$1"
    eval "$1"
    sleep "${2:-1.2}"
}

WORKDIR="$(mktemp -d)"
cd "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

sleep 0.6
run "texforge new quantum-paper" 1.5
run "cd quantum-paper && ls" 1.5
run "texforge check" 1.5
run "texforge fmt" 1.2
run "texforge build" 2.5
run "ls *.pdf" 2.5
sleep 1
