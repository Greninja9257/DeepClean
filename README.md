# DeepClean

A fast, deep macOS disk cleaner written in Rust, inspired by [Mole](https://github.com/tw93/Mole).
Scanning and deletion run in parallel across all cores (rayon work-stealing), so a full
cache scan usually finishes in under a second and a whole-home project sweep in a few seconds.

## The app

```sh
./build-app.sh                # builds build/DeepClean.app
./build-app.sh --install      # also copies it to /Applications
```

DeepClean.app is a SwiftUI front end over the same Rust engine (bundled inside the app).
It scans everything at once (caches, developer tools, project builds, Downloads, large files
and duplicates, leftovers from uninstalled apps, system items) and lets you tick what to clean.
System items ask for your password once. Grant the app Full Disk Access in System Settings
to also include Trash, app sandboxes and leftovers.

Building needs only the Xcode Command Line Tools and Rust.

## Command line

```sh
cargo install --path .        # puts `deepclean` in ~/.cargo/bin
```

## Usage

```sh
deepclean                     # interactive menu
deepclean clean               # caches, logs, trash, browser & dev-tool junk
deepclean purge [PATHS…]      # node_modules, target, .venv, build, … (default: ~)
deepclean installers          # .dmg/.pkg/.iso/.xip in Downloads & Desktop
sudo deepclean clean          # also /Library/Caches and system diagnostic reports
```

Common flags: `-n/--dry-run` (show, delete nothing), `-y/--yes` (skip the picker and clean
the recommended set), `--no-log`. `clean --containers` also scans sandboxed app containers
(macOS may ask for permission). `purge --days N` (default 7) skips recently-touched projects.

In the picker: <kbd>space</kbd> toggles, <kbd>a</kbd> toggles all, <kbd>enter</kbd> confirms,
<kbd>esc</kbd> cancels. ● items are pre-selected; ○ items are listed but opt-in.

## What it cleans

| Area | Examples |
| --- | --- |
| System | `~/Library/Caches`, `~/Library/Logs`, Trash, `~/.cache`, old `$TMPDIR` files (opt-in) |
| Browsers | Chrome, Brave, Edge, Arc, Vivaldi, Chromium (per-profile caches), Firefox, Safari |
| Apps | Electron/Chromium app caches (Slack, Discord, VS Code, Notion…), iOS update files |
| Xcode | DerivedData, DeviceSupport, simulator caches, unavailable simulators, SwiftPM |
| JavaScript | npm, Yarn, pnpm store, Bun, Deno, node-gyp, Electron, Playwright, Puppeteer, Cypress |
| Python | pip, uv, Poetry, pipenv, pre-commit, conda (`conda clean`); AI model downloads (opt-in) |
| Rust / Go / native | Cargo registry & git, rustup downloads, Go modules & build cache, ccache, sccache, Bazel, Homebrew |
| JVM | Gradle caches & wrappers, sbt/Ivy/Coursier, Kotlin/Native, Android, JetBrains; Maven (opt-in) |
| Other | CocoaPods, Carthage, Dart/Flutter, NuGet, Bundler, Composer, Hex, Cabal, Stack, Terraform, Docker prune (opt-in) |
| Projects (`purge`) | `node_modules`, `target`, `.gradle`, `build`, `.next`, `.nuxt`, `.svelte-kit`, `.turbo`, `__pycache__`, `.venv`, `Pods`, `.build`, `.dart_tool`, `zig-cache`, `.stack-work`, `dist-newstyle`, `_build`, .NET `bin`/`obj`, `.terraform`, `cmake-build-*`, … |

Project artifacts only match when the owning project's marker is present (e.g. `target` needs
`Cargo.toml`/`pom.xml`, `.venv` needs a `pyvenv.cfg` inside it).

## Safety

- Every path passes a safety gate right before deletion: it must be absolute and normalised,
  inside your home (or `/Library/Caches`, `/Library/Logs`, temp), never a top-level folder like
  `~/Documents`, and never inside `~/.ssh`, `~/.gnupg`, Keychains or iCloud Drive.
- Symlinks are removed, never followed.
- Broad sweeps such as `~/Library/Caches` empty each app's folder rather than deleting it.
- `purge` skips artifacts modified in the last 7 days, anything git-tracked, and folders holding
  keys/certificates (`.pem`, `.p12`, `.keystore`, …). `dist` and JS/Python `build` are opt-in.
- Items for apps that are running (Chrome, Xcode, …) are deselected by default.
- Non-regenerable things (iOS backups, Xcode Archives, Maven repo) are opt-in and labelled.
- Whitelist: `deepclean whitelist add <path>` / `remove` / `list`
  (stored in `~/.config/deepclean/whitelist`).
- Every operation is logged to `~/Library/Logs/deepclean/operations.log`; view with `deepclean history`.

Deletion is permanent (no Trash). Use `--dry-run` first if in doubt.
