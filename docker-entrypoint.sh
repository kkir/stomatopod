#!/bin/sh
set -e

# Mirrors EmbeddedConfig::default() ("./data"), which resolves to /app/data
# under this image's WORKDIR /app. STOMATOPOD_STORAGE__DATA_DIR overrides it.
DATA_DIR="${STOMATOPOD_STORAGE__DATA_DIR:-/app/data}"

if [ "$(id -u)" = "0" ]; then
    # Started as root (default): a bind-mounted host dir (e.g. Coolify) is
    # typically root-owned and hides the image's pre-chowned /app/data
    # regardless of mount path, so fix ownership here, then drop to the
    # unprivileged user before any app code runs.
    echo "stomatopod-entrypoint: running as root; ensuring ownership of ${DATA_DIR}"
    mkdir -p "${DATA_DIR}"
    owner="$(stat -c '%U:%G' "${DATA_DIR}" 2>/dev/null || echo '')"
    if [ "${owner}" != "stomatopod:stomatopod" ]; then
        echo "stomatopod-entrypoint: chown -R stomatopod:stomatopod ${DATA_DIR} (was '${owner:-unknown}')"
        chown -R stomatopod:stomatopod "${DATA_DIR}"
    fi
    exec gosu stomatopod "$@"
else
    # Operator/platform forced a non-root UID (e.g. --user): trust that they
    # provisioned correct ownership; just exec the command as-is.
    echo "stomatopod-entrypoint: running as uid $(id -u) (non-root); skipping chown, exec as-is"
    exec "$@"
fi
