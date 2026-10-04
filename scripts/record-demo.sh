#!/usr/bin/env bash
# Records assets/demo.gif from assets/demo.tape with VHS (brew install vhs).
#
# The recording runs a separate Herdr with its own config directory, HOME, and server, and
# installs the plugin from GitHub into it. Fake pbcopy/pbpaste keep the copies in a file, and a
# fake open/xdg-open launches nothing, so the live Herdr session, the real clipboard, and the
# browser stay untouched.
#
# Usage: scripts/record-demo.sh [tape]   (default: assets/demo.tape)
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
tape="${1:-$repo/assets/demo.tape}"
demo="$(mktemp -d /tmp/herdr-yank-demo.XXXXXX)"
demo_path="$demo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

# Runs herdr against the demo config only.
demo_herdr() {
    env -i HOME="$demo/home" PATH="$demo_path" XDG_CONFIG_HOME="$demo/config" herdr "$@"
}

cleanup() {
    demo_herdr server stop >/dev/null 2>&1 || true
    rm -rf "$demo"
}
trap cleanup EXIT

mkdir -p "$demo/bin" "$demo/home/web" "$demo/zdot" "$demo/config/herdr"
ln -s "$(command -v herdr)" "$demo/bin/herdr"

cat >"$demo/bin/pbcopy" <<EOF
#!/bin/sh
cat >"$demo/clipboard"
EOF
cat >"$demo/bin/pbpaste" <<EOF
#!/bin/sh
cat "$demo/clipboard" 2>/dev/null
EOF
printf '#!/bin/sh\nexit 0\n' >"$demo/bin/open"
cp "$demo/bin/open" "$demo/bin/xdg-open"

# The pane shell: zsh without the user's rc files, and a short prompt.
cat >"$demo/zdot/.zshrc" <<'EOF'
setopt interactivecomments
PROMPT_EOL_MARK=''
PROMPT='%F{blue}%~%f %F{green}❯%f '
EOF
cat >"$demo/bin/demo-shell" <<EOF
#!/bin/sh
cd "$demo/home/web"
exec env ZDOTDIR="$demo/zdot" /bin/zsh -i
EOF

# A deploy that prints the kind of tokens Herdr Yank gives hints to.
cat >"$demo/home/web/deploy.sh" <<'EOF'
#!/bin/sh
step() { printf '\033[1;34m→\033[0m \033[1m%-8s\033[0m%s\n' "$1" "$2"; sleep 0.2; }
info() { printf '          \033[2m%s\033[0m\n' "$1"; }
step build "main @ 9f3c2a1e7b4d"
info "modified:   src/routes/checkout.ts"
info "modified:   src/lib/cart.ts"
step rollout "deployment.apps/web in staging"
info "pod/web-7cf9dbd8c9-kx2p9   Running   10.42.0.17"
info "pod/web-7cf9dbd8c9-q8zt4   Running   10.42.0.18"
step trace "0d7d2d9e-0c72-4b8e-a2a9-8e7e5b4c92f1"
step logs "~/web/logs/deploy-1042.log"
printf '\033[1;32m✓\033[0m \033[1m%-8s\033[0m%s\n' live "https://staging.example.com/checkout?release=1042"
EOF

chmod +x "$demo/bin/pbcopy" "$demo/bin/pbpaste" "$demo/bin/open" "$demo/bin/xdg-open" \
    "$demo/bin/demo-shell" "$demo/home/web/deploy.sh"

cat >"$demo/config/herdr/config.toml" <<EOF
onboarding = false

[terminal]
default_shell = "$demo/bin/demo-shell"

[update]
version_check = false
manifest_check = false

[[keys.command]]
key = "prefix+y"
type = "plugin_action"
command = "pgilad.herdr-yank.yank"

[[keys.command]]
key = "prefix+shift+y"
type = "plugin_action"
command = "pgilad.herdr-yank.open-url"
EOF

demo_herdr plugin install pgilad/herdr-yank --yes >/dev/null
mkdir -p "$demo/config/herdr/plugins/config/pgilad.herdr-yank"
printf 'clipboard = "tool"\n' >"$demo/config/herdr/plugins/config/pgilad.herdr-yank/config.toml"

# The tape starts Herdr with exactly this environment.
export DEMO_HOME="$demo/home" DEMO_CONFIG="$demo/config" DEMO_PATH="$demo_path"
cd "$repo"
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION -u HERDR_ENV \
    -u HERDR_PANE_ID -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID vhs "$tape"
