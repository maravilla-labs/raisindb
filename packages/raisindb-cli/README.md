# @raisindb/cli

Interactive command-line interface for RaisinDB with a beautiful Ink-based terminal UI.

## Features

- Interactive shell with command history and autocomplete
- SQL mode with syntax highlighting and multiline support
- Browser-based authentication with OAuth flow
- Package management (create and upload .rap packages)
- Beautiful gradient banner and intuitive UI
- Configuration file support (.raisinrc)

## Installation

```bash
npm install -g @raisindb/cli
# or
pnpm add -g @raisindb/cli
```

## Usage

### Interactive Shell (Default)

Start the interactive shell:

```bash
raisindb
# or
raisindb shell
```

Connect to a specific server:

```bash
raisindb shell --server http://localhost:8080
```

Connect and use a specific database:

```bash
raisindb shell --server http://localhost:8080 --database mydb
```

### Package Commands (Offline)

Create a .rap package from a folder:

```bash
raisindb package create ./my-package
raisindb package create ./my-package --output my-package.rap
```

Upload a package to the server:

```bash
raisindb package upload my-package.rap
raisindb package upload my-package.rap --server http://localhost:8080
```

Keep environment-specific values out of the YAML with `{env:...}` tokens:

```yaml
# content/stories/my-site/.node.yaml
properties:
  dev_url: "{env:PREVIEW_SERVER:-http://localhost:5173}"
```

```bash
raisindb package create ./my-package                          # uses the default
PREVIEW_SERVER=https://preview.example.ch \
  raisindb package create ./my-package                        # uses the env value
raisindb package create ./my-package --env production         # reads .env.production
```

Values resolve from `.env`, `.env.<profile>`, `.env.local`, `--env-file`, and
the process environment (highest precedence). `.env*` files are never packaged
or pushed. A token with no value and no inline default fails the command
instead of shipping a literal. The same substitution applies to
`package validate`, `deploy`, `sync --push/--watch`, and `.raisin-sync.yaml`.

### Pulling content from a server

`raisindb pull` (the same as `raisindb sync --pull`) brings a package folder up
to date with what a server holds: pages, records, assets and translations that
editors changed on the live site land as the package files `deploy` installs.

```bash
raisindb pull ./package --repo website --dry-run       # what would change, nothing written
raisindb pull ./package --repo website                 # apply
raisindb pull ./package --repo website --diff          # also show a diff per changed file
raisindb pull ./package --path stories:/my-site        # only this subtree
raisindb pull ./package --path sponsors:/              # a whole workspace
```

- **Scope.** By default a pull refreshes what the package contains: the
  top-most nodes it defines in each `content/<workspace>/` directory and
  everything below them, at every depth. Top-level server nodes outside that
  are named, not pulled. `--path ws:/path` (repeatable) or `pull.roots` in
  `.raisindb-cli.yaml` replaces the default; `pull.exclude` takes the same specs.
- **Layout.** Nodes become `.node.yaml` (an existing `{name}.yaml` or
  `SKILL.md` is updated in place), asset bytes are streamed to disk and checked
  against the server's `content_hash`, translations become `.node.<locale>.yaml`
  holding only what the locale changes. References are written in path form;
  `$`/`__` bookkeeping is dropped. Renditions below an asset are skipped.
- **Your edits survive.** An existing file is updated value by value: comments,
  formatting and `{env:...}` tokens stay. A token matches any server value and
  is never replaced.
- **Who changed it.** `.raisindb/pull-state.json` records what the last pull
  wrote, so each file is new, updated (the server changed), a local edit (only
  you changed it; kept) or a conflict (both changed; also the case on a first
  pull when the files differ). Conflicts follow `conflict_strategy`
  (`prompt` asks at a terminal, `server`, `local`, `newest`); `--force` takes the
  server version; `--yes` never prompts. A second pull with nothing new writes
  nothing.
- **`.raisin-sync.yaml`.** Paths with `mode: replace` belong to the package (a
  deploy overwrites them), so the package is their source and pull skips them
  unless `--include-owned`. `direction: push_only`/`local_only` and `exclude`
  patterns are never pulled. The ignore list and `.rapignore` apply.
- **Deletions** are reported ("gone from server") and applied only with
  `--delete`.
- **Empty reads.** RaisinDB answers a read without valid credentials with an
  empty result, not 401. A pull that reads nothing stops with an error instead
  of reporting success (`--allow-empty` if the server really is empty), and a
  root that reads empty never marks local files as deleted.
- Node types, archetypes, element types, workspaces and the manifest are not
  pulled: the package is their source.

Pulling from another server, e.g. the live one, with its own credentials:

```bash
RAISINDB_TOKEN=<api key of the live repository> \
  raisindb pull ./package -s https://live.example.com --repo website --dry-run
```

`-s` beats `server:` in `.raisindb-cli.yaml`; `RAISINDB_TOKEN` is sent to
whatever server the command targets. (`RAISINDB_SERVER` does **not** override a
`.raisindb-cli.yaml`, and a `.raisinrc` is found by searching upward from the
current directory — pass `-s` to be certain.)

### Repositories and languages

A repository's default language is set when it is created. Base content is
stored in that language, and every other language is a translation overlay on
top of it, so pick it before the first install:

```bash
raisindb repo create website --default-language de --languages de,fr,en
raisindb repo languages website                  # show default + supported languages
raisindb repo languages website --add it         # add a translation language
raisindb repo languages website --default de     # change the default language
raisindb repo list                               # LANGUAGES column per repository
```

Without `--default-language` the server uses `en`, and `--languages` alone is
refused rather than guessing which entry is the base. `--exists-ok` fails when
the existing repository has a different default language, and warns about
missing translation languages.

`repo languages --default <code>` changes the default language. The base
content is not rewritten; it is from then on treated as the new language. The
new default is added to the supported languages and the old one kept. The
server refuses while translations in the new language exist (they would collide
with the base content), and otherwise queues a full-text rebuild of every
branch and prints the job ids. It asks for confirmation at a terminal; pass
`--yes` in scripts. After adding languages, rebuild the full-text index
(`POST /api/admin/management/database/{tenant}/{repo}/fulltext/rebuild`).

### Type catalogs

Translation files are checked against the `translatable` markers of the
archetype or element type they belong to, including fields inherited through
`extends`. Types the package does not ship itself (`extends:
standard:ContentPage`, `archetype: standard:ArticlePage`, a `standard:Hero`
block) are resolved from type catalogs, loaded offline:

- **builtin** — RaisinDB's own node types, generated into
  `dist/builtin-types.json` when the CLI is built
  (`scripts/build-builtin-catalog.mjs`).
- **`~/.raisindb/catalogs/*.json`** — e.g. the Studio catalog
  (`studio-types.json`) that `maravilla update` or a build server downloads.
  The CLI never fetches one itself; files that are not a
  `raisin-type-catalog` v1 JSON document are ignored.

`package validate` names the catalogs in use. Without a Studio catalog, keys
inherited from `standard:*` types are not checked.

### Agent Skills

Scaffold an agent skill as a `SKILL.md` (frontmatter + Markdown) inside a
package. `deploy --install` and `sync --push` both install it as a
`raisin:Skill` node named after its folder.

```bash
raisindb create skill pdf-forms                          # asks who gets it
raisindb create skill pdf-forms --scope package          # content/functions/skills/pdf-forms/SKILL.md
raisindb create skill pdf-forms --scope local            # content/functions/local/skills/pdf-forms/SKILL.md
raisindb create skill pdf-forms --scope agent --path lib/acme/skills
```

| Scope | Folder | Who gets it |
|-------|--------|-------------|
| `package` | `functions:/skills/<name>` | every agent, shipped with the package |
| `local` | `functions:/local/skills/<name>` | every agent, this installation only |
| `agent` | `functions:/<path>/<name>` | only agents that list it in `skills:` |

An agent opts out of the two global layers with `global_skills: false`.

## Shell Commands

### Connection & Authentication

- `/connect <url>` - Connect to a RaisinDB server
- `/login` - Authenticate via browser (OAuth flow)
- `/logout` - Clear stored authentication

### Database Operations

- `use <database>` - Switch to a different database
- `/databases` - List available databases

### SQL Mode

- `/sql` - Enter SQL mode for running queries
- `/exit-sql` - Exit SQL mode (also available in SQL mode)

In SQL mode:
- Type queries normally and press Enter for single-line queries
- Omit semicolon to start multiline mode
- Press `Ctrl+Enter` to execute multiline queries
- Press `ESC` to cancel multiline input

### Package Management

- `/packages` - List installed packages
- `/install <name>` - Install a package by name (installs mixins before node types)
- `/upload [file]` - Upload a package file

### Other Commands

- `/help` - Show help screen with all commands
- `/clear` - Clear the terminal screen
- `/quit` or `/exit` - Exit the CLI

## Configuration File

The CLI looks for `.raisinrc` in the current directory tree, falling back to `~/.raisinrc`.

Example `.raisinrc`:

```yaml
server: http://localhost:8080
token: your-auth-token
default_repo: mydb
```

Configuration is automatically updated when you use `/connect`, `/login`, or `use` commands.

## Server and Credentials

### Which server a command talks to

1. `-s, --server <url>` on the command
2. the `RAISINDB_SERVER` environment variable
3. `server` in `.raisinrc` (the server of the last `raisindb login`)
4. `http://localhost:8081`

`--server` applies to **every** request the command makes (for `deploy
--install`: the upload, the job stream, the install, the status polling and the
workspace reconcile). A login to another server never redirects it. `sync` and
`pull` use the server from `--server`, else `.raisindb-cli.yaml`, else the list
above.

Every command that writes (`deploy`, `package upload`/`install`, `sync`,
`repo create`/`delete`, `secret`, `cors`, `ai provider set`, `user register`)
prints its target on stderr before the first write:

```text
→ http://localhost:8080 (repo studio, branch main)
```

### Which token is sent

1. the `RAISINDB_TOKEN` environment variable, to whatever server the command
   targets (an explicit choice)
2. `token` in `.raisinrc`, **only** when the target is the server stored next
   to it. Servers are compared after normalizing scheme and host case, default
   ports and trailing slashes (`https://Host:443/` equals `https://host`).

`.raisinrc` holds one login (one server, one token). When the target is a
different server, the CLI sends no `Authorization` header and says so:

```text
Note: the saved login is for https://prod.example.com, not http://localhost:8080; sending no credentials.
  Log in to it with: raisindb login -s http://localhost:8080   (or set RAISINDB_TOKEN)
```

`deploy`, `package upload`/`install`/`list` and `sync` stop at that point with
the same hint instead of failing later on a 401. Logging in to a second server
replaces the saved login; use `RAISINDB_TOKEN` to work with two servers from
one shell.

## Development

Build the package:

```bash
pnpm build
```

Run in development mode:

```bash
pnpm dev
```

Run the built CLI:

```bash
pnpm start
```

## Architecture

The CLI is built with:

- **Ink** - React for terminal UIs
- **Commander** - CLI argument parsing
- **ink-text-input** - Interactive text input
- **ink-gradient** & **ink-big-text** - Beautiful banner
- **sql-highlight** - SQL syntax highlighting
- **yaml** - Configuration file parsing
- **open** - Browser launching for OAuth

## License

MIT
