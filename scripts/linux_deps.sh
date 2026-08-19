#!/usr/bin/env bash
# The system libraries Bevy links against, for a Linux CI runner.
#
# This exists because apt on a fresh runner has three ways to stop dead,
# and none of them ends on its own:
#
#   - a package mirror that accepts the connection and then says
#     nothing, which the Acquire timeouts below cut short;
#   - the dpkg lock, held by the unattended-upgrades run that starts
#     with the machine. No Acquire setting touches that one, so it gets
#     its own bound, and `timeout` sits outside both as the backstop;
#   - a mirror that is up but useless. This is the one that cost two
#     runs. The runner is pointed at a mirror near it, and when that
#     mirror stops serving, apt does not fail - it says `Ign` for every
#     index file in turn, each after its own retries and timeouts. Two
#     dozen files of that is minutes, and the attempt is killed before
#     it ever reaches the mirror that would have answered. The log shows
#     the canonical archive answering in the same breath.
#
# So the mirror the runner was given is put behind the canonical one
# rather than trusted, and apt is told not to retry a URL at all: this
# script's own loop is the retry, and it starts over from the top rather
# than grinding through a mirror that has already gone quiet.
set -uo pipefail

readonly PACKAGES=(pkg-config libasound2-dev libudev-dev)
readonly MIRRORS=/etc/apt/apt-mirrors.txt
readonly OPTIONS=(
  # One try per URL. Retrying inside apt only lengthens the wait on a
  # mirror that is not going to answer; the loop below is the retry.
  -o Acquire::Retries=0
  -o Acquire::http::Timeout=10
  -o Acquire::https::Timeout=10
  # A runner whose IPv6 route is a black hole is a common way for a
  # mirror to go quiet.
  -o Acquire::ForceIPv4=true
  # The lock the boot-time upgrade holds: wait a minute, not forever.
  -o DPkg::Lock::Timeout=60
)

# Ask the canonical archive first, and keep whatever the runner was
# pointed at as the fallback behind it.
if [ -f "$MIRRORS" ]; then
  preferred=$(printf 'http://archive.ubuntu.com/ubuntu\n%s\n' "$(cat "$MIRRORS")")
  printf '%s\n' "$preferred" | sudo tee "$MIRRORS" >/dev/null
  echo "mirrors, canonical first:" >&2
  sed 's/^/  /' "$MIRRORS" >&2
fi

for attempt in 1 2 3 4; do
  if sudo timeout 120 apt-get "${OPTIONS[@]}" update &&
    sudo timeout 180 apt-get "${OPTIONS[@]}" install \
      --no-install-recommends -y "${PACKAGES[@]}"; then
    exit 0
  fi
  echo "attempt $attempt did not get the packages; trying again" >&2
  # A mirror that has just gone quiet is not going to answer a second
  # later, so the wait grows.
  sleep $((attempt * 15))
done

echo "the packages could not be installed in four attempts" >&2
exit 1
