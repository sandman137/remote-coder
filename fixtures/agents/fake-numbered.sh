#!/usr/bin/env bash
# Fake agent: numbered-menu prompt (Cursor-style "1) apply 2) skip 3) abort").
set -u

i=0
while true; do
  i=$((i + 1))
  printf '\033[35m●\033[0m edit %d ready: src/main.rs (+12 -3)\n' "$i"
  sleep 0.4
  printf '1) apply  2) skip  3) abort\n> '
  IFS= read -r -n1 choice || exit 0
  printf '\n'
  case "$choice" in
    1) printf '\033[32mapplied.\033[0m\n' ;;
    2) printf 'skipped.\n' ;;
    3) printf '\033[31maborted.\033[0m\n' ;;
    *) printf 'unrecognized choice\n' ;;
  esac
  sleep 0.4
done
