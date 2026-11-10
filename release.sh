#!/bin/bash
# usage: ./release.sh <version|major|minor|patch>
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <version|major|minor|patch>" >&2
  exit 1
fi

cargo release --no-tag --no-push --no-publish --dependent-version upgrade --execute "$1"
version=$(cargo pkgid | sed 's/.*[#@]//')
git tag "$version"
# branch and tag in one push: both runs of the docker workflow see the same commit
git push --atomic origin HEAD "$version"
