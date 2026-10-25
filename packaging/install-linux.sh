#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Install an extracted Linux release for the current user, without sudo.
set -euo pipefail

package_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
if [[ "$data_dir" != /* || "$data_dir" == *$'\n'* ]]; then
    echo 'XDG_DATA_HOME must be an absolute path without newlines.' >&2
    exit 1
fi
install_dir="$data_dir/beamcraft"
for resource in beamcraft beamcraft-ui assets/icons/beamcraft-linux-256.png; do
    test -f "$package_dir/$resource" || { echo "Missing release resource: $resource" >&2; exit 1; }
done
case "$install_dir/" in
    "$package_dir/"*)
        if [[ "$install_dir" != "$package_dir" ]]; then
            echo 'Installation destination must not be inside the release folder.' >&2
            exit 1
        fi
        ;;
esac
mkdir -p "$install_dir"
if [[ "$package_dir" != "$install_dir" ]]; then
    cp -R "$package_dir/." "$install_dir/"
fi
chmod +x "$install_dir/beamcraft" "$install_dir/beamcraft-ui"
icon_dir="$data_dir/icons/hicolor/256x256/apps"
applications_dir="$data_dir/applications"
mkdir -p "$icon_dir" "$applications_dir"
cp "$install_dir/assets/icons/beamcraft-linux-256.png" "$icon_dir/com.apibim.BeamCraft.png"
# Desktop Exec uses its own quoting rules; it is not a shell command.
exec_path="$(printf '%s' "$install_dir/beamcraft-ui" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/`/\\`/g' -e 's/\$/\\$/g' -e 's/%/%%/g')"
cat > "$applications_dir/com.apibim.BeamCraft.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=BeamCraft
Comment=Structural steel detailing
Exec="$exec_path"
Icon=com.apibim.BeamCraft
Terminal=false
Categories=Graphics;Engineering;
StartupNotify=true
StartupWMClass=com.apibim.BeamCraft
DESKTOP
chmod 644 "$applications_dir/com.apibim.BeamCraft.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications_dir"
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -f "$data_dir/icons/hicolor/index.theme" ]]; then
    gtk-update-icon-cache -f -t "$data_dir/icons/hicolor"
fi
printf 'Installed BeamCraft. Open Show Applications, search for BeamCraft, then pin it to the Dock if wanted.\n'
