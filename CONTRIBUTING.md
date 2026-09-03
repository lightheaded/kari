# Contributing

kari is shaped by real needs. If you use it and something does not fit your work, open an issue and describe what you did and what you expected. A short description of a real session beats a feature list.

kari works with the tools people already use. It reads the state that Claude Code writes, and it talks to herdr through its socket. It does not replace those tools. A change that duplicates what one of them does well will not go in. A change that makes kari work with another tool of this kind is welcome.

One person maintains kari at the moment. Replies can take a few days.

## Before you write code

Open an issue first for anything larger than a fix. Say what problem you saw and how you want to solve it. This avoids work on a change that cannot go in.

## Development

Requirements: macOS 12 or later, Rust (`rustup`), Bun, `jq`.

```
bun install
bun tauri dev
```

Before you open a pull request, make sure that these pass:

```
bun run lint
bun run build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-privacy.sh
```

CI runs the same commands, plus a gitleaks scan of the history. The last script refuses absolute home paths, email addresses, private network addresses and internal host names. Use `~` or a placeholder such as `/Users/you/` instead.

## Pull requests

- Keep one change per pull request.
- Write the pull request text in plain, short sentences. Say what changed and why.
- Do not put personal data in fixtures, screenshots or examples: no real session ids, project paths, prompts or transcript excerpts.
- Sign off your commits with `git commit -s`. The sign-off says that you have the right to submit the change under the project license ([Developer Certificate of Origin](https://developercertificate.org/)).

## Screenshots

The README header and `TOUR.md` show the app as it looks in the current release. The images come from a script, not from a hand-made capture. Do not take screenshots of a real board for the docs.

- `scripts/demo-fixtures.mjs` writes a dummy board to `docs/demo/`. Every project, session, prompt and path in it is invented.
- `scripts/screenshots.mjs` starts the Vite dev server with that board, opens it in headless Chromium, and writes one PNG per view to `docs/screenshots/`. It also writes the app version to `docs/screenshots/VERSION`.
- `bun run demo` opens the same dummy board in your browser.

When you change the UI, run `bun run screenshots` and look at the result. Commit the new images with the change, or leave them for the next release. The release process retakes them in any case.

When you add a view, add a step to `scripts/screenshots.mjs`, add the file name to the check in `.github/workflows/release.yml`, and describe the view in `TOUR.md`.

The first run needs the Playwright Chromium build: `bunx playwright install chromium`.

## Releases

The maintainer cuts releases. The process:

1. Run `scripts/bump-version.sh X.Y.Z`. The script sets the version in the manifests, refreshes the lock files and retakes the screenshots.
2. Look at `docs/screenshots/` and at the README header. Every image must show this release.
3. Commit, tag `vX.Y.Z` with a signature (`git tag -s`) and push the tag.
4. The `release` workflow checks that the tag matches the app version and the screenshot stamp. Then it builds the macOS bundles and publishes the GitHub release.

The workflow refuses a tag whose version differs from `docs/screenshots/VERSION`. That keeps the README header at the latest release. Do not edit the stamp by hand. Run the script.

## License

Contributions are licensed under the Apache License 2.0, the same as the project. See `LICENSE`.
