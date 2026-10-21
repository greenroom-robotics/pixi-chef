#!/bin/sh
set -eu

stamps() {
    find "$PWD/.pixi/envs" -path '*/conda-meta/*.json' -exec stat -c '%n %i %Y' {} + | sort
}

stamps > /tmp/before
test -s /tmp/before
"$@"
stamps > /tmp/after

reinstalled=$(comm -23 /tmp/before /tmp/after)
if [ -n "$reinstalled" ]; then
    echo "binary packages were reinstalled:" >&2
    echo "$reinstalled" >&2
    exit 1
fi

stray=$(find / -xdev -type d -name conda-meta -not -path "$PWD/*" -not -path "/export/*" -not -path "$HOME/.cache/rattler/cache/backends-v0/*")
if [ -n "$stray" ]; then
    echo "environments created outside $PWD:" >&2
    echo "$stray" >&2
    exit 1
fi
