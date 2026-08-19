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
#     runs. The runner is given several mirrors in /etc/apt/apt-mirrors.txt
#     and prefers the one near it; when that one stops serving, apt does
#     not fail over quickly - it says `Ign` for each index file in turn,
#     each after its own retries and timeouts, and two dozen files of
#     that is minutes. The attempt is killed before apt reaches the
#     mirror that would have answered, and the log shows that mirror
#     answering in the same breath.
#
# The cure is to make the fall-through cheap rather than to choose the
# mirror: `Acquire::Retries=0` and a short timeout, so a mirror gone
# quiet costs one timeout per file instead of three, and this script's
# own loop starts over from the top.
#
# Choosing the mirror was tried and does not work from here. That file
# is read by priority, not by order - its lines carry `priority:1`,
# `priority:2` - so a line put at the top without one loses to the very
# mirror it was meant to replace. Said here so it is not tried twice.
set -uo pipefail

readonly PACKAGES=(pkg-config libasound2-dev libudev-dev)
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
