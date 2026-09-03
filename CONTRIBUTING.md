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
```

CI runs the same commands.

## Pull requests

- Keep one change per pull request.
- Write the pull request text in plain, short sentences. Say what changed and why.
- Do not put personal data in fixtures, screenshots or examples: no real session ids, project paths, prompts or transcript excerpts.
- Sign off your commits with `git commit -s`. The sign-off says that you have the right to submit the change under the project license ([Developer Certificate of Origin](https://developercertificate.org/)).

## Releases

The maintainer cuts releases. The process:

1. Run `scripts/bump-version.sh X.Y.Z`.
2. Commit, tag `vX.Y.Z` and push the tag.
3. The `release` workflow builds the macOS bundles and publishes the GitHub release.

## License

Contributions are licensed under the Apache License 2.0, the same as the project. See `LICENSE`.
