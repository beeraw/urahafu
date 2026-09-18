#!/bin/sh
# Signs Urahafu.app with the project's code signing certificate.
#
# macOS ties the Accessibility permission to the app's signing identity. With the project
# certificate, that identity is "com.beeraw.urahafu + this certificate", so the permission
# survives updates; with an ad-hoc signature it is the binary's own hash, and every new build
# loses it. See SECURITY.md ("Code signing") for how the certificate is kept.
#
# Usage: packaging/sign.sh path/to/Urahafu.app
#
# The identity is looked up by the certificate's SHA-1 fingerprint (public, also listed in the
# README). In CI (CI=true) a missing certificate is an error; locally, the script falls back to an
# ad-hoc signature with a warning, so contributors without the key can still build and run.

set -eu

APP="${1:?usage: packaging/sign.sh path/to/Urahafu.app}"
IDENTITY="${URAHAFU_SIGN_IDENTITY:-11700E5903A230D75A389FFD99928A689F5FA11A}"

if security find-identity -p codesigning | grep -q "$IDENTITY"; then
    codesign --force --deep --sign "$IDENTITY" "$APP"
elif [ "${CI:-}" = "true" ]; then
    echo "error: signing certificate $IDENTITY not found" >&2
    exit 1
else
    echo "warning: signing certificate not found, signing ad-hoc" >&2
    echo "warning: the Accessibility permission will not survive the next build" >&2
    codesign --force --deep --sign - "$APP"
fi

codesign --verify --strict "$APP"
codesign --display --requirements - "$APP" 2>&1 | grep 'designated'
