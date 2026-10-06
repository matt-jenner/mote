#!/bin/sh

: "${__NV_DISABLE_EXPLICIT_SYNC=1}"
export __NV_DISABLE_EXPLICIT_SYNC

exec /app/bin/mote-bin "$@"
