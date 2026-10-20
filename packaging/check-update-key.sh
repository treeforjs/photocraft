#!/usr/bin/env bash
# Validate the release signing pair, then optionally sign a checksum file.
set -euo pipefail
umask 077

fail() { printf '::error::%s\n' "$1" >&2; exit 1; }
[ -n "${UPDATE_SIGNING_KEY:-}" ] || fail 'Set UPDATE_SIGNING_KEY in the release environment.'
[ -n "${CRAFT_UPDATE_PUBLIC_KEY:-}" ] || fail 'Set the CRAFT_UPDATE_PUBLIC_KEY repository variable.'
[ "$#" -le 1 ] || fail 'Usage: check-update-key.sh [SHA256SUMS.txt]'

KEY_DIR=$(mktemp -d)
trap 'rm -rf "$KEY_DIR"' EXIT
printf '%s\n' "$UPDATE_SIGNING_KEY" >"$KEY_DIR/key.pem"
openssl pkey -passin pass: -in "$KEY_DIR/key.pem" -check -noout > /dev/null 2>&1 \
  || fail 'UPDATE_SIGNING_KEY must contain a valid unencrypted Ed25519 private PEM key.'
openssl pkey -passin pass: -in "$KEY_DIR/key.pem" -pubout -outform DER >"$KEY_DIR/actual.der" 2>/dev/null \
  || fail 'Could not derive the release public key.'
actual_hex=$(od -An -tx1 -v "$KEY_DIR/actual.der" | tr -d ' \n')
case "$actual_hex" in
  302a300506032b6570032100????????????????????????????????????????????????????????????????) ;;
  *) fail 'UPDATE_SIGNING_KEY must use Ed25519.' ;;
esac
printf '%s' "$CRAFT_UPDATE_PUBLIC_KEY" | base64 --decode >"$KEY_DIR/configured.pub" 2>/dev/null \
  || fail 'CRAFT_UPDATE_PUBLIC_KEY must contain base64 public-key bytes.'
[ "$(base64 -w0 "$KEY_DIR/configured.pub")" = "$CRAFT_UPDATE_PUBLIC_KEY" ] \
  || fail 'CRAFT_UPDATE_PUBLIC_KEY must contain single-line canonical base64.'
case "$(wc -c <"$KEY_DIR/configured.pub" | tr -d ' ')" in
  32) tail -c 32 "$KEY_DIR/actual.der" >"$KEY_DIR/expected.pub" ;;
  44) cp "$KEY_DIR/actual.der" "$KEY_DIR/expected.pub" ;;
  *) fail 'CRAFT_UPDATE_PUBLIC_KEY must encode an Ed25519 DER or raw public key.' ;;
esac
cmp -s "$KEY_DIR/configured.pub" "$KEY_DIR/expected.pub" \
  || fail 'CRAFT_UPDATE_PUBLIC_KEY does not match UPDATE_SIGNING_KEY.'

if [ "$#" -eq 1 ]; then
  [ -s "$1" ] || fail 'The checksum file is missing or empty.'
  openssl pkeyutl -sign -passin pass: -inkey "$KEY_DIR/key.pem" -rawin -in "$1" -out "$KEY_DIR/signature"
  openssl pkeyutl -verify -pubin -keyform DER -inkey "$KEY_DIR/actual.der" \
    -rawin -in "$1" -sigfile "$KEY_DIR/signature" > /dev/null
  base64 -w0 "$KEY_DIR/signature" >"$1.sig"
  test -s "$1.sig"
fi
