#!/usr/bin/env bash
# Gera os pacotes do launcher em pacote/saida/, com os nomes fixos que a
# página da organização usa (releases/latest/download/<nome>):
#   featherlauncher-linux-x86_64.tar.gz   Linux portátil (instalar.sh na pasta do usuário)
#   featherlauncher-linux-amd64.deb       Debian, Ubuntu, Mint…
#   featherlauncher-windows-x64.exe       Windows 10/11, portátil (sem instalador)
#   SHA256SUMS
#
# Uso: pacote/gerar.sh [linux|windows ...]   (sem argumentos = todos)
# Windows precisa do alvo x86_64-pc-windows-gnu e do mingw-w64.
set -euo pipefail

pacote="$(cd "$(dirname "$0")" && pwd)"
raiz="$(dirname "$pacote")"
versao="$(sed -n 's/^version = "\(.*\)"/\1/p' "$raiz/Cargo.toml" | head -1)"
saida="$pacote/saida"
alvos=("$@")
[[ ${#alvos[@]} -eq 0 ]] && alvos=(linux windows)
rm -rf "$saida" && mkdir -p "$saida"
echo "FeatherWorks Launcher $versao → $saida"

linux() {
  cargo build --release --manifest-path "$raiz/Cargo.toml"
  local bin="$raiz/target/release/featherlauncher"

  # .deb
  local d; d="$(mktemp -d)"
  install -Dm755 "$bin" "$d/usr/bin/featherlauncher"
  install -Dm644 "$pacote/featherlauncher.desktop" "$d/usr/share/applications/featherlauncher.desktop"
  for t in 16 24 32 48 64 128 256 512; do
    install -Dm644 "$pacote/icones/featherlauncher-$t.png" "$d/usr/share/icons/hicolor/${t}x${t}/apps/featherlauncher.png"
  done
  install -Dm644 "$pacote/icones/featherlauncher.svg" "$d/usr/share/icons/hicolor/scalable/apps/featherlauncher.svg"
  install -Dm644 "$raiz/LICENSE" "$d/usr/share/doc/featherlauncher/copyright"
  local tamanho; tamanho="$(du -sk "$d/usr" | cut -f1)"
  mkdir -p "$d/DEBIAN"
  cat > "$d/DEBIAN/control" <<EOF
Package: featherlauncher
Version: $versao
Section: utils
Priority: optional
Architecture: amd64
Installed-Size: $tamanho
Depends: libc6 (>= 2.39), libxkbcommon0, libwayland-client0 | libx11-6, libfontconfig1
Recommends: libvulkan1 | libgl1
Maintainer: FeatherWorks <https://github.com/feather-works>
Homepage: https://github.com/feather-works/featherlauncher
Description: Instalador da suíte FeatherWorks
 Instala, atualiza e abre o FeatherSheet, o FeatherSlide e os demais
 aplicativos da suíte, conferindo cada pacote pelo SHA256 da release.
EOF
  cat > "$d/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
command -v update-desktop-database >/dev/null && update-desktop-database -q /usr/share/applications || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
EOF
  cp "$d/DEBIAN/postinst" "$d/DEBIAN/postrm"
  chmod 755 "$d/DEBIAN/postinst" "$d/DEBIAN/postrm"
  dpkg-deb --root-owner-group --build "$d" "$saida/featherlauncher-linux-amd64.deb" >/dev/null
  rm -rf "$d"

  # .tar.gz portátil
  local t; t="$(mktemp -d)/featherlauncher-$versao"
  mkdir -p "$t"
  cp "$bin" "$t/featherlauncher"
  cp "$pacote/featherlauncher.desktop" "$raiz/LICENSE" "$t/"
  cp -r "$pacote/icones" "$t/"
  cat > "$t/instalar.sh" <<'EOF'
#!/bin/sh
# Instala o launcher só para este usuário (~/.local), sem administrador.
set -e
cd "$(dirname "$0")"
mkdir -p ~/.local/bin ~/.local/share/applications
install -m755 featherlauncher ~/.local/bin/featherlauncher
sed "s#^Exec=featherlauncher#Exec=$HOME/.local/bin/featherlauncher#" featherlauncher.desktop > ~/.local/share/applications/featherlauncher.desktop
for t in 16 24 32 48 64 128 256 512; do
  mkdir -p ~/.local/share/icons/hicolor/${t}x${t}/apps
  cp icones/featherlauncher-$t.png ~/.local/share/icons/hicolor/${t}x${t}/apps/featherlauncher.png
done
update-desktop-database ~/.local/share/applications 2>/dev/null || true
echo "Launcher instalado. Procure por \"FeatherWorks Launcher\" no menu de aplicativos."
EOF
  chmod 755 "$t/instalar.sh"
  tar -C "$(dirname "$t")" -czf "$saida/featherlauncher-linux-x86_64.tar.gz" "featherlauncher-$versao"
  rm -rf "$(dirname "$t")"
}

windows() {
  cargo build --release --target x86_64-pc-windows-gnu --manifest-path "$raiz/Cargo.toml"
  cp "$raiz/target/x86_64-pc-windows-gnu/release/featherlauncher.exe" "$saida/featherlauncher-windows-x64.exe"
  x86_64-w64-mingw32-strip "$saida/featherlauncher-windows-x64.exe" 2>/dev/null || true
}

for a in "${alvos[@]}"; do
  echo "== $a"
  "$a"
done
(cd "$saida" && sha256sum -- * | grep -v SHA256SUMS > SHA256SUMS)
ls -lh "$saida"
