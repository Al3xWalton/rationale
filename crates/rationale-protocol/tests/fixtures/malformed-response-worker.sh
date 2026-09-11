#!/bin/sh
read_frame() {
  header=$(dd bs=1 count=4 2>/dev/null | od -An -tu1)
  set -- $header
  length=$((($1 * 16777216) + ($2 * 65536) + ($3 * 256) + $4))
  dd bs=1 count="$length" of=/dev/null 2>/dev/null
}

read_frame
printf '\000\000\000\045{"kind":"ready","protocol_version":1}'
read_frame
printf '\000\000\000\001{'
sleep 5
