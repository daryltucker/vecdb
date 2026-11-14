#!/bin/bash
# Depth fixture: one binding or construct per addressable level, with
# distinct names so the depth test can ask "is X reachable?" per cell.
X=11
top_call "top-str"

f() {
  local y=33
  y=2
  if [ -n "$y" ]; then
    local b=44
    block_call "block-str"
  fi
  h() { :; }
  body_call "body-str"
  echo "$y"
}
