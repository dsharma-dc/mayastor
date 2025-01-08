#! /usr/bin/env bash

# Grab the arguments passed to the runner.
ARGS="${@}"

if [[ $EUID -ne 0 ]]; then
  MAYBE_SUDO='sudo -E --preserve-env=PATH'
else
  MAYBE_SUDO=''
fi

# Elevate to sudo so we can set some capabilities via `capsh`, then execute the args with the required capabilities:
#
# * Set `cap_setpcap` to be able to set [ambient capabilities](https://lwn.net/Articles/636533/) which can be inherited
# by children.
# * Set `cap_sys_admin,cap_ipc_lock,cap_sys_nice` as they are required by `mayastor`.
${MAYBE_SUDO} capsh \
  --caps="cap_setpcap+iep cap_sys_admin,cap_ipc_lock,cap_sys_nice+iep" \
  --addamb=cap_sys_admin --addamb=cap_ipc_lock --addamb=cap_sys_nice \
  -- -c "${ARGS}"
