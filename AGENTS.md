# Agents

- **A user wants hachikuji installed, or wants to replace Kiro CLI, Amazon Q Developer CLI, Fig or inshellisense:** follow [docs/MIGRATION.md](docs/MIGRATION.md). It lists the checks, the backups and the order of the steps.
- **You work on this repo:** read `README.md` (how it works, the tests) and `plan.md` (the hard constraints). Run `make test` before you commit.

## Git

- **Commit subjects and PR titles** use [Conventional Commits](https://www.conventionalcommits.org): `type(scope): summary`, for example `fix(shell): open the FIFOs non-blocking`. Types: `feat`, `fix`, `docs`, `test`, `build`, `ci`, `refactor`, `perf`, `chore`. The summary says what changed, not "address review comments". The `pr-title` check fails a PR whose title does not match, because the squash commit on `main` takes the PR title.
