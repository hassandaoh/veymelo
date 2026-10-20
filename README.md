# Veymelo

A video app for the Mac whose main user is an AI. You open the app and watch; an AI (Claude Code,
Claude Desktop or any app that speaks MCP) builds the video in it, one named command at a time,
and every change shows live: compositions, layers, keyframes, expressions, effects, 3D layers with
a camera, text, shapes, sound, and the render to MP4.

Written in Rust with no crates from outside: what the operating system brings (AppKit,
CoreGraphics, ImageIO, AVFoundation, Metal) and code of its own for everything else.

## What it needs

- A Mac with Apple silicon (M1 or later), macOS 11 or later
- Rust (stable, edition 2024) to build it

## Build and run

```sh
cargo run --release          # opens the app
cargo test --workspace       # every test
scripts/bundle.sh            # dist/Veymelo.app and dist/Veymelo.dmg
```

## Connect an AI

The app has a "Connect an AI" card that sets this up for you. By hand, for Claude Code:

```sh
claude mcp add --scope user veymelo -- /Applications/Veymelo.app/Contents/MacOS/veymelo mcp --bridge
```

`veymelo mcp --bridge` works in the project open in the app, so the person watches the AI work;
`veymelo mcp` works on its own, without a window. From the shell, `veymelo commands` lists what
there is and `veymelo describe COMMAND` tells what a command takes.

## Projects

A project is a folder with a `.veymelo` file in it (JSON, readable by people and AIs), its
`assets/` and its `renders/`. Double-click the file in Finder to open it in the app.

## More

- `PLAN.md`: how the program is put together and what comes next (in Thai)
- `NOTICE`: what was learned from [EffectCraft](https://github.com/storytold/effectcraft) and where
