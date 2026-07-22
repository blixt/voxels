#!/bin/sh
set -eu

if [ "$(id -u)" = "0" ]; then
  chown voxels:voxels /data
  exec gosu voxels:voxels "$0" "$@"
fi

exec /usr/local/bin/voxels-worldd /app/config/world-service.toml "$@"
