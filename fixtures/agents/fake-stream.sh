#!/usr/bin/env bash
# Fake agent: continuous colored output with cursor movement and periodic
# alt-screen interludes — exercises the VT grid path (256-color, truecolor,
# bold/reverse, cursor addressing, line rewrite). DESIGN.md §10.2.
set -u

i=0
while true; do
  i=$((i + 1))
  printf '\033[38;5;%dmstream tick %d\033[0m\n' $(((i % 200) + 16)) "$i"
  printf '\033[38;2;255;128;0mtruecolor\033[0m \033[1mbold\033[0m \033[7mreverse\033[0m\n'
  sleep 0.5
  if ((i % 12 == 0)); then
    # Brief alt-screen interlude with absolute cursor addressing.
    printf '\033[?1049h\033[2J\033[H'
    printf '\033[5;10H\033[33m[alt-screen interlude %d]\033[0m' "$i"
    sleep 1.2
    printf '\033[?1049l'
  fi
  if ((i % 5 == 0)); then
    # Rewrite the previous line in place (cursor-up + erase-line).
    printf '\033[1A\r\033[Krewritten line %d\n' "$i"
  fi
done
