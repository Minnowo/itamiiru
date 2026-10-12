#!/usr/bin/env bash
# Development install: builds from this checkout and installs for your user,
# leaving the session environment alone. Re-run after every rebuild you want
# other apps to pick up. See install.sh for the details and --uninstall.
exec "$(dirname "$0")/install.sh" --no-env "$@"
