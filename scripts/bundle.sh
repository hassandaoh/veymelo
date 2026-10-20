#!/bin/sh
# Veymelo.app, and a disk image to hand it on in: the app, with the command line tool AIs reach
# it through (Contents/MacOS/veymelo: `veymelo mcp --bridge`) beside it, built for every Mac with
# Apple silicon (M1 and every chip after it) on macOS 11 or later. Signed for this Mac only (ad
# hoc): on another Mac it opens the first time with Control-click → Open.
#
#   scripts/bundle.sh            → dist/Veymelo.app and dist/Veymelo.dmg
#   scripts/bundle.sh --no-dmg   → dist/Veymelo.app only
#
# Only tools that come with macOS (and cargo) are used.
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p veymelo-studio -p veymelo-cli

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=dist/Veymelo.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/veymelo-studio "$app/Contents/MacOS/veymelo-studio"
cp target/release/veymelo "$app/Contents/MacOS/veymelo"

# the icon, drawn by the app itself at every size
icons=$(mktemp -d)/Veymelo.iconset
"$app/Contents/MacOS/veymelo-studio" --icon "$icons"
iconutil -c icns "$icons" -o "$app/Contents/Resources/Veymelo.icns"
rm -rf "$(dirname "$icons")"

# what it is; and its projects (.veymelo files) are its own, so Finder opens them in it
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>Veymelo</string>
	<key>CFBundleDisplayName</key>
	<string>Veymelo</string>
	<key>CFBundleIdentifier</key>
	<string>app.veymelo.studio</string>
	<key>CFBundleExecutable</key>
	<string>veymelo-studio</string>
	<key>CFBundleIconFile</key>
	<string>Veymelo</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSArchitecturePriority</key>
	<array>
		<string>arm64</string>
	</array>
	<key>LSRequiresNativeExecution</key>
	<true/>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.video</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>Veymelo</string>
	<key>UTExportedTypeDeclarations</key>
	<array>
		<dict>
			<key>UTTypeIdentifier</key>
			<string>app.veymelo.project</string>
			<key>UTTypeDescription</key>
			<string>Veymelo Project</string>
			<key>UTTypeConformsTo</key>
			<array>
				<string>public.json</string>
			</array>
			<key>UTTypeTagSpecification</key>
			<dict>
				<key>public.filename-extension</key>
				<array>
					<string>veymelo</string>
				</array>
			</dict>
		</dict>
	</array>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Veymelo Project</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>app.veymelo.project</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# signed as it is (ad hoc): the tool first, then the app round it
codesign --force --sign - "$app/Contents/MacOS/veymelo"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
echo "made $app ($(du -sh "$app" | cut -f1 | tr -d ' '))"

if [ "${1:-}" != "--no-dmg" ]; then
	# the app beside a way to Applications, to drag it there
	stage=$(mktemp -d)
	cp -R "$app" "$stage/"
	ln -s /Applications "$stage/Applications"
	rm -f dist/Veymelo.dmg
	hdiutil create -quiet -volname Veymelo -srcfolder "$stage" -ov -format UDZO dist/Veymelo.dmg
	rm -rf "$stage"
	echo "made dist/Veymelo.dmg ($(du -sh dist/Veymelo.dmg | cut -f1 | tr -d ' '))"
fi
