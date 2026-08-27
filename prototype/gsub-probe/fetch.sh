#!/bin/sh
# PROTOTYPE — throwaway.  Downloads the ligature fonts the probe measures.
# Fonts are not committed; this re-fetches them.  Needs `gh`.
#
# Every face, not just Regular: the renderer draws four styles, and a bold
# ligature glyph is not the regular one scaled.
set -e
dir="$(dirname "$0")/fonts"
mkdir -p "$dir"
cd "$dir"

# The four faces we care about, and the weights we do not.
KEEP='-(Regular|Bold|Italic|BoldItalic|It|BoldIt|Oblique)\.(ttf|otf)$|Var( Italic)?\.ttf$'
DROP='Thin|ExtraLight|Light|Medium|SemiBold|ExtraBold|Black|Retina|Mono|NL-'

get() {  # repo  asset-glob  path-fragment-inside-zip
    repo="$1"; asset="$2"; want="$3"
    name=$(basename "$repo")
    [ -n "$(ls "$name"-* 2>/dev/null)" ] && return 0
    echo "fetching $repo …"
    rm -rf .work && mkdir .work
    gh release download --repo "$repo" --pattern "$asset" --dir .work --clobber
    (cd .work && unzip -qo ./*.zip)
    find .work -type f \( -name '*.ttf' -o -name '*.otf' \) \
        | grep -E -e "$want" | grep -E -e "$KEEP" | grep -Ev -e "$DROP" \
        | while read -r f; do
            cp "$f" "./$name-$(basename "$f" | tr ' ' '_')"
        done
    [ -z "$(ls "$name"-* 2>/dev/null)" ] && echo "  !! nothing matched $want"
    rm -rf .work
}

get tonsky/FiraCode         'Fira_Code_v*.zip'     '/ttf/FiraCode-'
get i-tu/Hasklig            'Hasklig-*.zip'        '/Hasklig-'
get githubnext/monaspace    'monaspace-variable-*' 'Monaspace Neon Var'
get be5invis/Iosevka        'PkgTTF-Iosevka-*.zip' '/Iosevka-'
get microsoft/cascadia-code 'CascadiaCode-*.zip'   '/ttf/static/Cascadia'

echo "--- fonts/"
ls -1 *.ttf *.otf 2>/dev/null | sed 's/^/  /'
