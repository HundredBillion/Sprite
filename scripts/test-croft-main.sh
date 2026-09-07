#!/usr/bin/env bash
# Build unmodified upstream Croft from its moving `main` and run Sprite's
# external capability smoke against it.
#
# This is the explicit network-enabled external phase. Ordinary Rust tests never
# call this script, and Croft is never a Sprite runtime dependency: it is an
# acceptance application that happens to exercise a real terminal hard.

set -euo pipefail

CROFT_REPO="https://github.com/vitali87/croft.git"
CROFT_BRANCH="main"

# Resolve the workspace from this script's own location, so the script works
# from any working directory.
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace="$(cd -- "${script_dir}/.." && pwd)"
cd "${workspace}"

mkdir -p target
commit_file="${workspace}/target/croft-main-commit.txt"
log_file="${workspace}/target/croft-main.log"

work_dir="$(mktemp -d)"
cleanup() {
  rm -rf "${work_dir}"
}
trap cleanup EXIT

echo "cloning ${CROFT_REPO} (${CROFT_BRANCH})"
git clone --depth 1 --branch "${CROFT_BRANCH}" "${CROFT_REPO}" "${work_dir}/croft"

croft_sha="$(git -C "${work_dir}/croft" rev-parse HEAD)"
printf '%s\n' "${croft_sha}" > "${commit_file}"
echo "croft main is ${croft_sha}"

# Croft builds with its own committed lockfile and its own toolchain settings.
# Sprite's --locked --offline flags belong to Sprite's workspace, not this one.
(
  cd "${work_dir}/croft"
  cargo build --release --locked
) 2>&1 | tee "${log_file}"

croft_bin="${work_dir}/croft/target/release/croft"
if [[ ! -x "${croft_bin}" ]]; then
  echo "croft binary not found at ${croft_bin}" | tee -a "${log_file}"
  exit 1
fi

# `tee` would otherwise mask a failing test, so the pipeline's own status is
# what decides the exit code.
set -o pipefail
SPRITE_CROFT_BIN="${croft_bin}" \
  cargo test -p sprite-term --test croft_smoke --locked --offline \
  -- --ignored --exact croft_checkpoint_one_capabilities \
  2>&1 | tee -a "${log_file}"
test_status=${PIPESTATUS[0]}

# An unmodified upstream is part of what this proves: a Croft tree dirtied by
# the run invalidates the result.
if [[ -n "$(git -C "${work_dir}/croft" status --porcelain)" ]]; then
  echo "croft has a tracked diff after the run; the smoke must not modify it" \
    | tee -a "${log_file}"
  exit 1
fi

exit "${test_status}"
