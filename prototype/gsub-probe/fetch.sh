#!/bin/sh
# PROTOTYPE — throwaway. Downloads the ligature fonts the probe measures.
# Fonts are not committed; this re-fetches them.  Needs `gh`.
set -e
dir="$(dirname "$0")/fonts"
mkdir -p "$dir"
cd "$dir"

get() {  # repo  asset-glob  ttf-glob-inside-zip
    repo="$1"; asset="$2"; want="$3"
    name=$(basename "$repo")
    { [ -f "$name.ttf" ] || [ -f "$name.otf" ]; } && return 0
    echo "fetching $repo …"
    rm -rf .work && mkdir .work
    gh release download --repo "$repo" --pattern "$asset" --dir .work --clobber
    (cd .work && unzip -qo ./*.zip)
    found=$(find .work -type f \( -name '*.ttf' -o -name '*.otf' \) -path "$want" | head -1)
    [ -z "$found" ] && { echo "  !! nothing matched $want"; return 0; }
    case "$found" in *.otf) ext=otf;; *) ext=ttf;; esac
    cp "$found" "./$name.$ext"
    rm -rf .work
}

get tonsky/FiraCode       'Fira_Code_v*.zip'      '*/ttf/FiraCode-Regular.ttf'
get i-tu/Hasklig          'Hasklig-*.zip'         '*Hasklig-Regular.otf'
get githubnext/monaspace  'monaspace-variable-*'  '*Monaspace Neon Var.ttf'
get be5invis/Iosevka      'PkgTTF-Iosevka-*.zip'  '*Iosevka-Regular.ttf'
get microsoft/cascadia-code 'CascadiaCode-*.zip'  '*/ttf/static/CascadiaCode-Regular.ttf'

echo "--- fonts/"
ls -lh *.ttf *.otf 2>/dev/null | awk '{print "  " $5 "  " $9}'
