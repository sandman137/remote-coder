#!/usr/bin/env bash
# Fake agent: prints "work", then blocks on a y/n prompt — the stand-in for
# Claude Code / Codex approval flows. Deterministic and offline (DESIGN.md §10.2).
set -u

i=0
while true; do
  i=$((i + 1))
  printf '\033[36m●\033[0m working on task %d…\n' "$i"
  sleep 0.4
  printf '\033[32m✓\033[0m step complete (tokens: %d)\n' $((i * 137))
  sleep 0.2
  printf 'Proceed? (y/n) '
  IFS= read -r -n1 ans || exit 0
  printf '\n'
  case "$ans" in
    y | Y) printf '\033[32mproceeding…\033[0m\n' ;;
    n | N) printf '\033[31mstep aborted.\033[0m\n' ;;
    *) printf 'unrecognized input\n' ;;
  esac
  sleep 0.4
done
