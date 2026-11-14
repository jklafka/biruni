#!/usr/bin/env bash
# Copy the Linux screenshots from a PR's latest CI run into
# visual-tests/baselines/ and commit them as you.
#
#   scripts/update-baselines.sh <PR number> [--push]
#
# Run it from a clean checkout of the PR's branch, after CI has finished on the
# commit you want to promote. Needs the GitHub CLI (`gh`), logged in.
set -euo pipefail

usage() {
  echo "usage: $0 <PR number> [--push]" >&2
  exit 2
}

pr=""
push=false
for arg in "$@"; do
  case "$arg" in
    --push) push=true ;;
    -*) usage ;;
    *) if [ -z "$pr" ]; then pr="$arg"; else usage; fi ;;
  esac
done
[[ "$pr" =~ ^[0-9]+$ ]] || usage

cd "$(git rev-parse --show-toplevel)"

read -r head_sha head_ref < <(gh pr view "$pr" --json headRefOid,headRefName \
  --jq '[.headRefOid, .headRefName] | @tsv')

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" != "$head_ref" ]; then
  echo "error: PR #$pr is on branch '$head_ref' but you are on '$branch'. Check it out first." >&2
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "error: working tree is not clean. Commit or stash your changes first." >&2
  exit 1
fi
if [ "$(git rev-parse HEAD)" != "$head_sha" ]; then
  echo "warning: local HEAD differs from the PR head (${head_sha:0:7}); the screenshots may not match your checkout." >&2
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Newest completed CI runs for the PR's head commit first; take the first one
# that has the screenshots artifact.
run_ids=$(gh run list --workflow ci.yml --commit "$head_sha" --status completed \
  --limit 10 --json databaseId --jq '.[].databaseId')
found=""
for id in $run_ids; do
  if gh run download "$id" -n visual-screenshots -D "$tmp" 2>/dev/null; then
    found="$id"
    break
  fi
done
if [ -z "$found" ]; then
  echo "error: no completed CI run with a 'visual-screenshots' artifact for ${head_sha:0:7} yet." >&2
  echo "Wait for CI to finish on the latest push (gh pr checks $pr --watch), then try again." >&2
  exit 1
fi
echo "Using CI run $found for ${head_sha:0:7}"

# The artifact holds actual/ and diff/ under the common root of the upload.
shopt -s nullglob
pngs=("$tmp"/actual/*.png "$tmp"/visual-tests/actual/*.png)
if [ ${#pngs[@]} -eq 0 ]; then
  echo "error: artifact contains no actual/*.png" >&2
  exit 1
fi
cp "${pngs[@]}" visual-tests/baselines/

if [ -z "$(git status --porcelain -- visual-tests/baselines)" ]; then
  echo "Baselines already match CI; nothing to commit."
  exit 0
fi
echo "Changed baselines:"
git status --short -- visual-tests/baselines

git add visual-tests/baselines
git commit -q -m "Update visual baselines from Linux CI" \
  -m "Screenshots from CI run $found."
echo "Committed."
if $push; then
  git push
else
  echo "Review the PNGs, then run: git push"
fi
