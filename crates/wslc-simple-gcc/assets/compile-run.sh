set -euo pipefail
compiler=$1
language=$2
standard=$3
shift 3

version=$("$compiler" -dumpfullversion -dumpversion)
if [[ ${version%%.*} != 16 ]]; then
    printf 'Expected GCC 16, found %s\n' "$version" >&2
    exit 2
fi
printf '[GCC %s | %s]\n' "$version" "$language ${standard}" >&2

count=$1
shift
sources=("${@:1:count}")
shift "$count"
count=$1
shift
flags=("${@:1:count}")
shift "$count"

standard_flags=()
if [[ -n $standard ]]; then
    standard_flags=("-std=$standard")
fi
build_dir=$(mktemp -d /tmp/wsl-gcc.XXXXXXXX)
"$compiler" -x "$language" "${standard_flags[@]}" -Wall -Wextra "${sources[@]}" "${flags[@]}" -o "$build_dir/program"
exec "$build_dir/program" "$@"
