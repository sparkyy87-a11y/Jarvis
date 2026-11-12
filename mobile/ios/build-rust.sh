#!/usr/bin/env bash
# Baut den Rust-Kern für iPhone/iPad (Gerät + Simulator) als XCFramework und
# erzeugt die Swift-Anbindung. Läuft auf einem Mac mit Xcode.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=mobile/ios/Generated
TARGETS="aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios"

rustup target add $TARGETS
for t in $TARGETS; do
  cargo build --release -p mira-mobile --lib --target "$t"
done

rm -rf "$OUT" && mkdir -p "$OUT/include" "$OUT/sim" "$OUT/bindings"
# Anbindung aus einem ungestrippten Host-Build erzeugen (Metadaten sind plattformunabhängig).
cargo build -q -p mira-mobile --lib
cargo run -q -p uniffi-bindgen -- generate --library target/debug/libmira_mobile.dylib \
  --language swift --out-dir "$OUT/bindings"
mv "$OUT/bindings/mira_mobile.swift" "$OUT/"
mv "$OUT/bindings/mira_mobileFFI.h" "$OUT/include/"
mv "$OUT/bindings/mira_mobileFFI.modulemap" "$OUT/include/module.modulemap"
rmdir "$OUT/bindings"

lipo -create \
  target/aarch64-apple-ios-sim/release/libmira_mobile.a \
  target/x86_64-apple-ios/release/libmira_mobile.a \
  -output "$OUT/sim/libmira_mobile.a"
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libmira_mobile.a -headers "$OUT/include" \
  -library "$OUT/sim/libmira_mobile.a" -headers "$OUT/include" \
  -output "$OUT/MiraCore.xcframework"
echo "Fertig: $OUT/MiraCore.xcframework"
