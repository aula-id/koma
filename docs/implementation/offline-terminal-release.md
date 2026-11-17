# Offline terminal release

Release notes: Koma now carries its complete OpenTUI terminal, including Bun
1.4.3 and native/parser dependencies. Opening the downloaded executable works
offline, with local extraction on first launch. Downloads are larger. macOS 13+
is required by Bun for the new terminal; legacy and other Rust entrypoints retain
the existing deployment target. Linux release compilation remains on Ubuntu
22.04 (glibc 2.35). Installer asset names and dependency behavior are unchanged.

## Preparation and validation

Run `python3 scripts/prepare-terminal.py [--target TRIPLE]`, then build with Cargo.
The same command is used by `build.sh`, CI and releases. The manifest records the
Rust target, Bun archive checksum/version, installed dependency versions, all
terminal input hashes, archive checksum and per-file SHA-256/permissions. Bun
archives are pinned against upstream bun-v1.4.3 SHASUMS256.txt. Production installs
use a fresh directory, frozen lockfile, explicit OS/CPU and disabled lifecycle
scripts. Developer node_modules never enters the payload.

`--check` validates build inputs without downloading anything. Cargo runs that
check before its GUI feature branch and registers terminal source/asset trees,
lockfile, package/TypeScript manifests, preparation script and prepared payload
as rebuild inputs. Unsupported targets fail preparation explicitly.

At runtime, the SHA-256 of the embedded manifest names the cache directory.
A persistent per-payload lock file uses an OS advisory lock, which releases on
process death. Cold extraction verifies each file in a temporary sibling before
renaming it into place. An interrupted staging directory is discarded under the
lock. Cached launches verify contents and executable permissions; repairs run
under the same lock. Existing versions keep separate directories, allowing an
older executable to launch after an upgrade. No cache eviction is performed.
`KOMA_TERM_DIR` explicitly opts into developer sources and dependencies; it does
not select a different runtime. There is no automatic checkout or system Bun
search and no automatic legacy fallback.

## Release gates

`koma --terminal-self-test` uses the production extraction and runtime paths.
It checks the runtime version, native rendering/resizing, worker startup and all
shipped grammar/query assets. The command is hidden from normal CLI help.

`python3 scripts/verify-terminal-release.py PATH_TO_BINARY` copies the executable
to a temporary directory with spaces, isolates user data, empties PATH, removes
development overrides and blocks outbound networking through the OS. Linux needs
`sudo -n unshare --net`; macOS uses `sandbox-exec`; Windows needs administrator
firewall access and `pywinpty`. These are test prerequisites, not user runtime
requirements. The harness checks cold/cached extraction and corrupt/missing cache
recovery, then exercises normal and hub startup through PTY/ConPTY, keyboard,
resize down to the narrow-screen warning and back, and clean terminal restoration. It uses a local model/provider fixture and makes no live provider requests.

Release builds upload workflow artifacts only. Matching native runners execute
all five Unix artifacts (Linux x64, the headless Linux x64 build, Linux ARM64,
macOS ARM64 and macOS Intel), including the cross-built Linux ARM64 and macOS
Intel binaries. The headless build must also link no GUI libraries.
Windows executes its final binary before packaging; the administrative MSI
extraction must contain the identical SHA-256 executable. Frozen Bun tests,
TypeScript checks and Rust extraction tests run on all five architectures.
Only the final publish job can attach release assets, and it requires all builds
and verification jobs. Workflow dispatch prepares and verifies artifacts without
publishing; tagging/publishing needs separate user authorization.

Local verification cannot substitute for native runner results on other targets.
Do not describe an unrun workflow as a verified five-platform release.

## Local verification for this change

On macOS ARM64, the complete frozen Bun suite passed (405 tests), TypeScript
passed, production Clippy passed with warnings denied, and the Rust extraction
regression passed. A GUI-enabled optimized executable passed the copied-binary
network-denied harness. Other platform execution and MSI checks are configured
as release gates and require their native runners; they were not run locally.
