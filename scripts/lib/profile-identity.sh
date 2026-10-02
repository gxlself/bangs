# Sourced by the iCloud scripts. profile_identity <profile> prints the SHA-1 of
# the first signing identity in the keychain that the provisioning profile
# lists among its developer certificates, or nothing when there is none.
profile_identity() {
  local identities
  identities="$(security find-identity -v -p codesigning 2>/dev/null)"
  security cms -D -i "$1" 2>/dev/null | /usr/bin/python3 -c '
import hashlib, plistlib, sys
for certificate in plistlib.loads(sys.stdin.buffer.read()).get("DeveloperCertificates", []):
    print(hashlib.sha1(certificate).hexdigest().upper())' 2>/dev/null |
    while read -r sha; do
      awk -v sha="$sha" '$2 == sha { print $2; exit }' <<<"$identities"
    done | sed -n 1p
}
