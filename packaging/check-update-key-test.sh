#!/usr/bin/env bash
# Exercise release signing with throwaway keys; no repository secrets are used.
set -euo pipefail
umask 077
CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-update-key.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

openssl genpkey -algorithm ED25519 -out "$WORK/key.pem" 2>/dev/null
openssl pkey -in "$WORK/key.pem" -pubout -outform DER >"$WORK/public.der"
export UPDATE_SIGNING_KEY CRAFT_UPDATE_PUBLIC_KEY
UPDATE_SIGNING_KEY=$(cat "$WORK/key.pem")
CRAFT_UPDATE_PUBLIC_KEY=$(base64 -w0 "$WORK/public.der")
printf 'fixture\n' >"$WORK/package.zip"
(cd "$WORK" && sha256sum package.zip >SHA256SUMS.txt)
bash "$CHECK" "$WORK/SHA256SUMS.txt" >"$WORK/valid.log" 2>&1
if grep -Fq -f "$WORK/key.pem" "$WORK/valid.log"; then
  echo 'Signing output exposed private key contents' >&2
  exit 1
fi
base64 --decode "$WORK/SHA256SUMS.txt.sig" >"$WORK/signature"
openssl pkeyutl -verify -pubin -keyform DER -inkey "$WORK/public.der" \
  -rawin -in "$WORK/SHA256SUMS.txt" -sigfile "$WORK/signature" > /dev/null

CRAFT_UPDATE_PUBLIC_KEY=$(tail -c 32 "$WORK/public.der" | base64 -w0)
bash "$CHECK"

refuse() {
  if bash "$CHECK" >"$WORK/error.log" 2>&1; then
    echo "Unexpectedly accepted $1" >&2
    exit 1
  fi
  if grep -Fq -f "$WORK/key.pem" "$WORK/error.log"; then
    echo 'Validation output exposed private key contents' >&2
    exit 1
  fi
}
CRAFT_UPDATE_PUBLIC_KEY='' refuse 'an empty public key'
UPDATE_SIGNING_KEY='' refuse 'an empty private key'
CRAFT_UPDATE_PUBLIC_KEY='not base64' refuse 'malformed public key bytes'
valid_public=$CRAFT_UPDATE_PUBLIC_KEY
CRAFT_UPDATE_PUBLIC_KEY="${valid_public:0:16}"$'\n'"${valid_public:16}" refuse 'a wrapped public key'
CRAFT_UPDATE_PUBLIC_KEY=" $valid_public" refuse 'public key whitespace'
CRAFT_UPDATE_PUBLIC_KEY="$valid_public"$'\n' refuse 'a public key with a trailing newline'
UPDATE_SIGNING_KEY='-----BEGIN PRIVATE KEY-----' refuse 'an incomplete private key'
openssl genpkey -algorithm ED25519 -out "$WORK/other.pem" 2>/dev/null
CRAFT_UPDATE_PUBLIC_KEY=$(openssl pkey -in "$WORK/other.pem" -pubout -outform DER | base64 -w0)
refuse 'a mismatched public key'
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$WORK/rsa.pem" 2>/dev/null
UPDATE_SIGNING_KEY=$(cat "$WORK/rsa.pem") refuse 'a different key algorithm'

printf 'changed\n' >>"$WORK/SHA256SUMS.txt"
if openssl pkeyutl -verify -pubin -keyform DER -inkey "$WORK/public.der" \
  -rawin -in "$WORK/SHA256SUMS.txt" -sigfile "$WORK/signature" > /dev/null 2>&1; then
  echo 'A modified checksum file passed signature verification' >&2
  exit 1
fi
echo 'Update signing checks passed'
