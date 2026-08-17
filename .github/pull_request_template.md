## Summary

<!-- Briefly describe the changes made in this pull request and the motivation behind them. -->

## Changes

- 
- 

## Type of Change

- [ ] ⚡ `perf`: Performance improvement (benchmarks required below)
- [ ] ✨ `feat`: New feature or command support
- [ ] 🐛 `fix`: Bug fix
- [ ] ♻️ `refactor`: Code refactoring without behavior change
- [ ] 🧪 `test`: Adding or improving tests
- [ ] 📝 `docs`: Documentation updates

## Performance & Benchmark Impact

<!--
FyroDB follows a strict Zero-Regression Policy.
If this PR touches execution hot paths, data structures, or memory management, provide before/after benchmark results.
-->

- **Benchmark Command**: `task bench-fyrodb` / `task bench-key-fyrodb`
- **Before**: `... ops/sec`
- **After**: `... ops/sec`
- **Binary Size Impact**: `...` (if applicable)

## Checklist

- [ ] Code compiles without warnings (`cargo clippy -- -D warnings` / `task lint`).
- [ ] Tests pass locally (`cargo test` / `task test`).
- [ ] Formatted with `cargo fmt`.
- [ ] Documentation updated if relevant (e.g. `ARCHITECTURE.md`, `README.md`, `docs/`).
- [ ] Verified no performance regression on affected paths.
