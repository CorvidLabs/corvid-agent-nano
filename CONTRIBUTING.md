# Contributing to corvid-agent-nano

Thank you for your interest in contributing to corvid-agent-nano! This document provides guidelines and instructions for contributors.

## Code of Conduct

Be respectful, inclusive, and professional. We're building a welcoming community.

## Ways to Contribute

- **Report bugs** — Use GitHub Issues with clear reproduction steps
- **Suggest features** — Discuss new capabilities in Issues before implementing
- **Write code** — Fix bugs, implement features, improve performance
- **Improve documentation** — Fix typos, clarify guides, add examples
- **Review PRs** — Help maintain code quality and consistency
- **Write plugins** — Create and share example WASM plugins

## Before You Start

1. **Check existing issues/PRs** — Avoid duplicate work
2. **Read the docs** — Understand the architecture and design
3. **Read the specs** — Check `specs/` for detailed module specifications
4. **Understand the layout**:
   - `src/` — Main CLI binary
   - `crates/` — Plugin system, SDK, and tooling
   - `plugins/` — Example plugins
   - `docs/src/` — mdBook documentation
   - `specs/` — Module specifications (source of truth)

## Development Setup

### Prerequisites

- **Rust** — 1.75+ ([rustup](https://rustup.rs))
- **Algorand** — AlgoKit for localnet ([algokit.io](https://algokit.io))
- **Bun** — For plugin bridge tests (optional)

### Initial Setup

```bash
# Clone the repo
git clone https://github.com/CorvidLabs/corvid-agent-nano.git
cd corvid-agent-nano

# Start localnet
algokit localnet start

# Build and test
cargo build
cargo test
cargo fmt
cargo clippy
```

## Verification Before Committing

Always run the full verification suite:

```bash
# Lint with Biome and Clippy
cargo fmt              # Format code
cargo clippy           # Lint check
cargo build --release  # Full build

# Type check and tests
cargo test             # Run all tests

# Spec validation
specsync check         # Validate specs (requires specsync CLI)
```

All checks must pass before opening a PR.

## Making Changes

### 1. Create a branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/your-bug-name
```

Branch naming conventions:
- `feature/` — New features
- `fix/` — Bug fixes
- `docs/` — Documentation improvements
- `perf/` — Performance improvements
- `refactor/` — Code cleanup (no behavior changes)

### 2. Implement your changes

**Rules:**
- Follow Rust idioms and the existing code style
- Use meaningful variable and function names
- Add comments for non-obvious logic
- Keep functions focused and testable
- Update specs in `specs/` if your change affects module behavior

**For documentation changes:**
- Test build with `mdbook build docs/`
- Run spell checker: `mdspell docs/src/**/*.md` (if installed)
- Link to relevant docs/API from your changes

**For plugin-related changes:**
- Update both SDK and example plugins if applicable
- Test with the hello-world plugin: `cargo build --target wasm32-wasip1 --release`
- Ensure plugin host can load and invoke your changes

### 3. Write tests

- Add unit tests in the same file as your code (Rust convention)
- Add integration tests in `*/tests/` directories
- For plugins, add tests in `crates/corvid-plugin-host/src/tests/`
- Aim for high coverage on new code paths

Example test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_your_feature() {
        // Arrange
        let input = "test";

        // Act
        let result = your_function(input);

        // Assert
        assert_eq!(result, "expected");
    }
}
```

### 4. Update documentation

- **If you changed CLI behavior:** Update the relevant doc in `docs/src/commands/`
- **If you added a CLI command:** Add doc file following the template
- **If you changed the architecture:** Update `docs/src/architecture/`
- **If you fixed a common issue:** Add to `docs/src/reference/troubleshooting.md`
- **For major changes:** Update README.md and/or introduction.md

### 5. Commit your changes

Use clear, descriptive commit messages:

```bash
git commit -m "feat: add plugin reload command

- Adds --reload flag to plugin load command
- Unloads existing plugin with same ID before reloading
- Fixes issue #123"
```

Commit message format:
- `feat:` — New feature
- `fix:` — Bug fix
- `docs:` — Documentation
- `test:` — Tests
- `perf:` — Performance
- `refactor:` — Code cleanup
- `chore:` — Maintenance

Reference issues when relevant: `Fixes #123` or `Closes #456`

## Opening a Pull Request

### Before submitting:

1. Rebase onto latest `main`
2. Run the full verification suite (see above)
3. Check the PR is focused on one change (split large changes into multiple PRs)
4. Ensure commit history is clean and descriptive

### PR template:

```markdown
## Description

Brief description of what this PR does.

## Changes

- Change 1
- Change 2
- Change 3

## Testing

How did you test these changes?

## Checklist

- [ ] Ran `cargo fmt`, `cargo clippy`
- [ ] Ran `cargo test`
- [ ] Ran `specsync check`
- [ ] Updated docs (if needed)
- [ ] Commit messages are clear
```

### PR review expectations:

- **Tests pass** — All CI checks must be green
- **Code style** — Follows Rust conventions
- **Documentation** — Changes are documented
- **Specs updated** — If behavior changed, specs are updated
- **No breaking changes** — Without discussion (bump MSRV carefully)

## Documentation Contributions

Documentation improvements are highly valued! Here's how:

1. **mdBook guides** — Add to `docs/src/` (use markdown)
2. **README** — Update for major feature changes
3. **CHANGELOG** — Add to "Unreleased" section with your changes
4. **In-code docs** — Keep doc comments accurate
5. **Examples** — Add working code examples to guides

Build and test docs locally:

```bash
mdbook build docs/
mdbook serve docs/  # Open http://localhost:3000
```

## Creating Plugins

To contribute an example plugin:

1. **Create a new directory** under `plugins/`:
   ```bash
   cargo new plugins/your-plugin --lib
   cd plugins/your-plugin
   ```

2. **Add the SDK as dependency** in `Cargo.toml`:
   ```toml
   [package]
   name = "your-plugin"
   version = "0.1.0"

   [lib]
   crate-type = ["cdylib"]

   [dependencies]
   corvid-plugin-sdk = { path = "../../crates/corvid-plugin-sdk" }
   ```

3. **Implement your plugin** in `src/lib.rs`:
   ```rust
   use corvid_plugin_sdk::prelude::*;

   #[corvid_plugin]
   struct YourPlugin;

   #[corvid_tool(name = "your-tool", description = "What it does")]
   fn your_tool(input: YourInput) -> YourOutput {
       // Implementation
   }
   ```

4. **Build and test**:
   ```bash
   cargo build -p your-plugin --target wasm32-wasip1 --release
   ```

5. **Document** — Add a guide to `docs/src/guides/` explaining what your plugin does and how to use it

## Getting Help

- **Questions?** — Open a Discussion on GitHub
- **Stuck?** — Comment on an Issue or PR and ask for help
- **Design questions?** — Discuss in Issues before implementing
- **Documentation unclear?** — File an issue — help us improve!

## Code Review Tips

When reviewing PRs:
- Check tests pass and coverage is high
- Verify specs are up-to-date
- Look for security issues (especially around crypto, network, file access)
- Suggest improvements politely
- Acknowledge good work!

## Release Process

(For maintainers)

1. Update version in `Cargo.toml` and `Cargo.lock`
2. Update CHANGELOG.md with release notes
3. Tag the commit: `git tag v0.x.y`
4. Push: `git push origin main --tags`
5. Publish to crates.io: `cargo publish`
6. Create GitHub Release with CHANGELOG excerpt

## Thank You

Thank you for contributing to corvid-agent-nano! 🦅
