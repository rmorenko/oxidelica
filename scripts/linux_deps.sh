#!/usr/bin/env bash
# The system libraries Bevy links against, for a Linux CI runner.
#
# This exists because apt on a fresh runner has two ways to stop dead,
# and neither of them ends on its own:
#
#   - a package mirror that accepts the connection and then says
#     nothing, which the Acquire timeouts below cut short;
#   - the dpkg lock, held by the unattended-upgrades run that starts
#     with the machine. No Acquire setting touches that one, so it gets
#     its own bound, and `timeout` sits outside both as the backstop.
#
# A stalled attempt is therefore killed rather than waited on, and the
# retry is what actually gets the packages - which is what was observed
# on the runner that recovered while its neighbour hung for six
# minutes. Put the whole thing here rather than in the workflow, since
# two jobs need it and one copy is enough.
set -uo pipefail

readonly PACKAGES=(pkg-config libasound2-dev libudev-dev)
readonly OPTIONS=(
  -o Acquire::Retries=3
  -o Acquire::http::Timeout=15
  -o Acquire::https::Timeout=15
  # A runner whose IPv6 route is a black hole is a common way for a
  # mirror to go quiet.
  -o Acquire::ForceIPv4=true
  # The lock the boot-time upgrade holds: wait a minute, not forever.
  -o DPkg::Lock::Timeout=60
)

for attempt in 1 2 3; do
  if sudo timeout 90 apt-get "${OPTIONS[@]}" update &&
    sudo timeout 180 apt-get "${OPTIONS[@]}" install \
      --no-install-recommends -y "${PACKAGES[@]}"; then
    exit 0
  fi
  echo "attempt $attempt did not get the packages; trying again" >&2
  sleep 10
done

echo "the packages could not be installed in three attempts" >&2
exit 1
