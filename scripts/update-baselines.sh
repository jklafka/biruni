#!/usr/bin/env bash
# Copy the Linux screenshots from a PR's latest CI run into
# visual-tests/baselines/ and commit them as you.
#
#   scripts/update-baselines.sh <PR number> [--push] [--all]
#   scripts/update-baselines.sh --run <CI run id> [--all]   # copy only: no PR checks, no commit
#
# Only PNGs that differ from the committed baseline (any pixel over the visual
# test's per-pixel threshold, or a size change) or are new are copied. CI
# renders unchanged views with different bytes and tiny pixel noise, and those
# files are skipped so they don't make noisy commits. --all copies every PNG.
#
# Run it from a clean checkout of the PR's branch, after CI has finished on the
# commit you want to promote. Needs the GitHub CLI (`gh`), logged in, and Node.
set -euo pipefail

usage() {
  echo "usage: $0 <PR number> [--push] [--all] | $0 --run <CI run id> [--all]" >&2
  exit 2
}

pr=""
push=false
run=""
all=false
while [ $# -gt 0 ]; do
  case "$1" in
    --push) push=true ;;
    --all) all=true ;;
    --run)
      [ $# -ge 2 ] || usage
      run="$2"
      shift
      ;;
    -*) usage ;;
    *) if [ -z "$pr" ]; then pr="$1"; else usage; fi ;;
  esac
  shift
done
if [ -n "$run" ]; then
  if ! [[ "$run" =~ ^[0-9]+$ ]] || [ -n "$pr" ] || $push; then usage; fi
else
  [[ "$pr" =~ ^[0-9]+$ ]] || usage
fi

cd "$(git rev-parse --show-toplevel)"

if [ ! -d visual-tests/node_modules/pngjs ]; then
  echo "Installing visual-tests dependencies (npm ci)..."
  (cd visual-tests && npm ci --silent)
fi

head_sha=""
if [ -z "$run" ]; then
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
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -n "$run" ]; then
  if ! gh run download "$run" -n visual-screenshots -D "$tmp"; then
    echo "error: could not download a 'visual-screenshots' artifact from run $run." >&2
    exit 1
  fi
  found="$run"
  echo "Using CI run $found"
else
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
fi

# The artifact holds actual/ and diff/ under the common root of the upload.
shopt -s nullglob
pngs=("$tmp"/actual/*.png "$tmp"/visual-tests/actual/*.png)
if [ ${#pngs[@]} -eq 0 ]; then
  echo "error: artifact contains no actual/*.png" >&2
  exit 1
fi

# Copy a screenshot only if it is new or its decoded pixels differ from the
# committed baseline (same-pixels.mjs exits 0 for identical pixels, 1 for
# different, 2 on error, which aborts under set -e via the explicit check).
for png in "${pngs[@]}"; do
  name=$(basename "$png")
  dest="visual-tests/baselines/$name"
  if [ ! -e "$dest" ]; then
    cp "$png" "$dest"
    echo "  new: $name"
    continue
  fi
  if $all; then
    cp "$png" "$dest"
    echo "  copied (--all): $name"
    continue
  fi
  rc=0
  detail=$(node visual-tests/same-pixels.mjs "$png" "$dest") || rc=$?
  case "$rc" in
    0) echo "  unchanged pixels, skipped: $name" ;;
    1)
      cp "$png" "$dest"
      echo "  changed ($detail): $name"
      ;;
    *)
      echo "error: could not compare $name" >&2
      exit 1
      ;;
  esac
done

if [ -n "$run" ]; then
  echo "Copied only; nothing committed. Review with: git status"
  exit 0
fi

if [ -z "$(git status --porcelain -- visual-tests/baselines)" ]; then
  echo "Baselines already match CI; nothing to commit."
  exit 0
fi

git add visual-tests/baselines
git commit -q -m "Update visual baselines from Linux CI" \
  -m "Screenshots from CI run $found."
echo "Committed."
if $push; then
  git push
else
  echo "Review the PNGs, then run: git push"
fi
