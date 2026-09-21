#!/usr/bin/env bash
# A certificate for testing on a phone over the local network.
#
# Why this is needed at all: Safari hands out the motion sensors only in a
# secure context. `http://192.168.x.x:1425` is not one, so the living eye
# cannot work there no matter what is allowed — the request is refused and not
# a single orientation event arrives. localhost is a secure context, https is
# a secure context, a plain LAN address is not.
#
# The certificate is self-signed, so Safari will warn once; open the warning,
# "Show details", "visit this website". After that the page is https, the
# context is secure, and the sensor can be asked for.
#
# Usage: bash scripts/dev-cert.sh   then   npm run dev:phone

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERT_DIR="$REPO_ROOT/.certs"
mkdir -p "$CERT_DIR"

# Every address the phone might use has to be in the certificate, or Safari
# refuses it outright instead of merely warning.
LAN_IP="$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true)"
ALT="DNS:localhost,IP:127.0.0.1,IP:::1"
if [ -n "$LAN_IP" ]; then
    ALT="$ALT,IP:$LAN_IP"
    echo "==> Including this Mac's address on the network: $LAN_IP"
else
    echo "==> No address found on en0/en1; the certificate will cover localhost only" >&2
fi

openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$CERT_DIR/key.pem" \
    -out "$CERT_DIR/cert.pem" \
    -days 365 \
    -subj "/CN=ProxysVPN dev" \
    -addext "subjectAltName=$ALT" \
    -addext "basicConstraints=CA:FALSE" \
    2>/dev/null

chmod 600 "$CERT_DIR/key.pem"

WHERE="${LAN_IP:-this-Mac-on-the-network}"

cat <<EOF

Done. The certificate is in .certs (ignored by git, valid for a year).

  npm run dev:phone

On the phone open

  https://$WHERE:1425/

Safari warns about the certificate once: "Show details", then "visit this
website". After that the page is https, so the context is secure. Tap anywhere
on it - the sensor is asked for on the first gesture - and allow motion access.

Plain "npm run dev" stays on http, which is what tauri dev expects.
EOF
