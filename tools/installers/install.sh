#!/bin/sh
set -eu
prefix="$HOME/.local/bin"
no_plugin=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { echo '--prefix needs a directory' >&2; exit 2; }
            prefix=$2; shift 2 ;;
        --no-plugin) no_plugin=true; shift ;;
        *) echo "Unknown option: $1 (use --prefix DIR or --no-plugin)" >&2; exit 2 ;;
    esac
done
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p -- "$prefix"
prefix=$(CDPATH= cd -- "$prefix" && pwd)
install -m 755 -- "$source_dir/axiom" "$prefix/axiom"

setup_plugin() {
    markets=$(claude plugin marketplace list --json) || return 1
    if printf '%s\n' "$markets" | grep -Eq '"name"[[:space:]]*:[[:space:]]*"axiom-chronos"'; then
        claude plugin marketplace update axiom-chronos || return 1
    else
        claude plugin marketplace add xinhi6716/axiom-chronos || return 1
    fi
    # update is scoped to the user; install handles a missing user installation.
    claude plugin update chronos@axiom-chronos --scope user ||
        claude plugin install chronos@axiom-chronos --scope user
}
status=0
if [ "$no_plugin" = false ] && command -v claude >/dev/null 2>&1; then
    if ! setup_plugin; then
        echo 'Programs installed, but the Claude plugin setup failed.' >&2
        status=1
    fi
fi
echo "Installed Axiom Chronos in $prefix"
case ":$PATH:" in
    *":$prefix:"*) ;;
    *) echo "Add $prefix to PATH in your shell configuration." ;;
esac
echo 'Health check (axiom doctor):'
if "$prefix/axiom" doctor; then
    :
else
    echo 'axiom doctor reported problems. The program is installed.' >&2
fi
exit "$status"
