# Agents

- **A user wants hachikuji installed, or wants to replace Kiro CLI, Amazon Q Developer CLI, Fig or inshellisense:** follow [docs/MIGRATION.md](docs/MIGRATION.md). It lists the checks, the backups and the order of the steps.
- **You work on this repo:** read `README.md` (how it works, the tests) and `plan.md` (the hard constraints). Run `make test` before you commit.

## Git

- **Commit subjects and PR titles** use [Conventional Commits](https://www.conventionalcommits.org): `type(scope): summary`, for example `fix(shell): open the FIFOs non-blocking`. Types: `feat`, `fix`, `docs`, `test`, `build`, `refactor`, `perf`, `chore`. The summary says what changed, not "address review comments".
- **`main` accepts only squash merges.** The squash commit takes the PR title as its subject and the PR body as its message, so the PR title must be a conventional commit too.
- One topic per PR. Branch from `main`; GitHub deletes the branch after the merge.
